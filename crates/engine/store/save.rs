//! Read, write, migrate and check the save.

use std::collections::BTreeSet;

use serde::Deserialize;
use serde_saphyr::SerializerOptions;

use crate::canon::{unknown_item, ItemDef};
use crate::diagnostic::{check_tag, error, locate_nth, parse_file, schema_error, Diagnostic};
use crate::entity;
use crate::state::component::meter::{self, Meter};
use crate::state::component::modifier::{self, Modifier};
use crate::state::id::{ActorId, GlobalId};
use crate::state::{nested_map, Save, State};
use crate::store::records::{AddedMeter, DroppedMeter, MigrateRecord};
use crate::Reason;

const FILE: &str = "state/save.yaml";

pub(crate) fn emit_options() -> SerializerOptions {
    serde_saphyr::ser_options! {
        empty_as_braces: true,
        indent_step: 2,
        compact_list_indent: false,
        anchor_generator: None,
        min_fold_chars: usize::MAX,
        folded_wrap_chars: usize::MAX,
        tagged_enums: false,
        prefer_block_scalars: false,
        quote_all: false,
        yaml_12: false,
    }
}

pub fn parse(text: &str) -> Result<Save, Vec<Diagnostic>> {
    if let Some(diag) = check_tag(text, crate::diagnostic::SAVE_SCHEMA, FILE, 0) {
        return Err(vec![diag]);
    }
    // Check the rules version before the full shape, so a save from older rules
    // (for example one that still carries `presence`) is refused as RULES_REV_MISMATCH.
    #[derive(Deserialize)]
    struct RulesProbe {
        #[serde(default)]
        rules_rev: u64,
    }
    if let Ok(probe) = serde_saphyr::from_str::<RulesProbe>(text) {
        if super::require_rules_rev(probe.rules_rev).is_err() {
            return Err(vec![rules_rev_diagnostic(text)]);
        }
    }
    parse_file(FILE, text, 0)
}

pub fn emit(save: &Save) -> Result<String, Reason> {
    let mut canonical = save.clone();
    canonical.schema = crate::diagnostic::SAVE_SCHEMA.to_string();
    canonical.rules_rev = super::RULES_REV;
    canonical.sort();
    let text = serde_saphyr::to_string_with_options(&canonical, emit_options())
        .map_err(|_| Reason::Schema)?;
    Ok(one_trailing_lf(text))
}

fn rules_rev_diagnostic(text: &str) -> Diagnostic {
    let mut diag = error(
        FILE,
        Reason::RulesRevMismatch,
        "this save was written by different rules",
        "start a fresh save",
    )
    .at_path("rules_rev");
    if let Some((line, column)) = locate_nth(text, "rules_rev:", 0) {
        diag = diag.at_line(line, Some(column));
    }
    diag
}

fn one_trailing_lf(mut text: String) -> String {
    while text.ends_with('\n') {
        text.pop();
    }
    text.push('\n');
    text
}

/// Meters the pack gives an entity that the entity lacks, each at its `start`.
pub(crate) fn find_missing_meters(state: &State) -> Vec<AddedMeter> {
    let canon = state.canon();
    let defs = canon.meters.iter().chain(&canon.world_meters);
    defs.filter(|(id, def)| !find_lacking_carriers(state, &def.carrier, id).is_empty())
        .map(|(id, def)| AddedMeter::new(id, def))
        .collect()
}

/// The entities `carrier` names that lack meter `id`. Every-actor and one-actor carriers are read
/// off the save's actors, so a cast member the save does not list names nobody; any other carrier
/// is itself. Migrate and `check_save` both read it, so they agree on who lacks a meter.
pub(crate) fn find_lacking_carriers(
    save: &Save,
    carrier: &Option<entity::Id>,
    id: &str,
) -> Vec<entity::Id> {
    let held = match carrier {
        None | Some(entity::Id::Actor(_)) => {
            let actors = save.actors();
            let named = |actor: &&ActorId| match carrier {
                Some(entity::Id::Actor(one)) => one == *actor,
                _ => true,
            };
            let lacks = |actor: &&ActorId| nested_map::get(&actors.meters, actor, id).is_none();
            let lacking = actors.kind.keys().filter(named).filter(lacks).cloned();
            return lacking.map(entity::Id::Actor).collect();
        }
        Some(entity::Id::Place(place)) => nested_map::get(&save.places().meters, place, id),
        Some(entity::Id::Story(story)) => nested_map::get(&save.stories().meters, story, id),
        Some(entity::Id::Global) => nested_map::get(&save.globals().meters, &GlobalId, id),
    };
    held.is_none()
        .then(|| carrier.clone())
        .flatten()
        .into_iter()
        .collect()
}

/// Every meter the save lists, with the actor, place, story or world that holds it.
fn held_meters(save: &Save) -> impl Iterator<Item = (entity::Id, &String, &Meter)> {
    let actors = save.actors().meters.iter();
    let places = save.places().meters.iter();
    let stories = save.stories().meters.iter();
    let world = save.globals().meters.iter();
    (actors.map(|((id, key), held)| (entity::Id::Actor(id.clone()), key, held)))
        .chain(places.map(|((id, key), held)| (entity::Id::Place(id.clone()), key, held)))
        .chain(stories.map(|((id, key), held)| (entity::Id::Story(id.clone()), key, held)))
        .chain(world.map(|((_, key), held)| (entity::Id::Global, key, held)))
}

/// Meters an actor, place, story or the world lists that the pack no longer gives it.
pub(crate) fn find_stale_meters(state: &State) -> Vec<DroppedMeter> {
    held_meters(state)
        .filter(|(carrier, id, _)| state.canon().meter_def(carrier.as_ref(), id).is_none())
        .map(|(carrier, id, _)| DroppedMeter {
            id: id.clone(),
            carrier,
        })
        .collect()
}

/// Each actor's modifiers that the pack no longer defines, or now defines on another meter:
/// the actor and modifier id, with the modifier as the save holds it.
pub(crate) fn find_stale_modifiers(state: &State) -> Vec<(&(ActorId, String), &Modifier)> {
    let held = state.actors().stores().modifiers.iter();
    held.filter(|((_, id), live)| !modifier::is_defined(&state.canon().modifiers, id, live))
        .collect()
}

/// Migrate a save as `record` says, through the writers a round uses: give each entity the
/// meters it lacks, at their `start`, take the dropped modifiers off every actor and each dropped
/// meter off its entity, then stamp the record's `pack_rev_after` and `state_rev_after`. A meter
/// an entity holds keeps its value. Live migrate and replay both call this. It does not touch the
/// disk, and it reads no hash or chain from the record.
pub fn migrate(mut state: State, record: &MigrateRecord) -> Result<State, Reason> {
    for meter in &record.meters {
        let start = Meter {
            base: meter.start,
            offset: 0,
        };
        for id in find_lacking_carriers(&state, &meter.carrier, &meter.id) {
            let mut carrier = state.entity_mut(&id);
            carrier.set_meter(&meter.id, Some(start.clone()))?;
        }
    }
    let modifiers = state.actors().stores().modifiers.keys();
    let dropped = |(_, id): &&(ActorId, String)| record.dropped_modifiers.contains(id);
    let held: Vec<(ActorId, String)> = (modifiers.filter(dropped)).cloned().collect();
    for (actor, id) in &held {
        state.actors_mut().set_modifier(actor, id, None)?;
    }
    for meter in &record.dropped_meters {
        state
            .entity_mut(&meter.carrier)
            .set_meter(&meter.id, None)?;
    }
    state.pack_rev = record.pack_rev_after.clone();
    state.state_rev = record.state_rev_after;
    state.accept();
    Ok(state)
}

/// Check the save against the pack without changing it. Refuse every component on an actor
/// with no kind, every actor without a place or an action state, every actor in a place or on
/// an exit the pack lacks, every meter, item and wielded slot the pack does not define, every
/// meter the pack gives an entity that the entity lacks, every meter outside its rules, every
/// modifier the pack no longer gives that meter, and every empty id or zero count.
pub fn check_save(state: &State) -> Result<(), Vec<Diagnostic>> {
    let canon = state.canon();
    let mut diags = Vec::new();
    let actors = state.actors().stores();
    // Every store, with no `..`: a store added later must be listed here to compile.
    let crate::state::entity::Actors {
        kind: _,
        place,
        wielded,
        known_facts,
        visited,
        acts,
        items,
        meters,
        modifiers,
        flags,
    } = actors;
    let held: BTreeSet<(&str, &ActorId)> = (place.keys().map(|id| ("place", id)))
        .chain(wielded.keys().map(|id| ("wielded", id)))
        .chain(known_facts.keys().map(|id| ("known_facts", id)))
        .chain(visited.keys().map(|(id, _)| ("visited", id)))
        .chain(acts.keys().map(|id| ("acts", id)))
        .chain(items.keys().map(|(id, _)| ("items", id)))
        .chain(meters.keys().map(|(id, _)| ("meters", id)))
        .chain(modifiers.keys().map(|(id, _)| ("modifiers", id)))
        .chain(flags.keys().map(|(id, _)| ("flags", id)))
        .collect();
    for (store, id) in held.iter().filter(|(_, id)| !actors.kind.contains_key(*id)) {
        let problem = format!("actor '{id}' has {store} but no kind");
        let diag = schema_error(FILE, &problem, "give the actor a kind under actors.kind");
        diags.push(diag.at_path(format!("actors.{store}.{id}")));
    }
    for id in actors.kind.keys() {
        for store in ["place", "acts"] {
            if !held.contains(&(store, id)) {
                let problem = format!("actor '{id}' has a kind but no {store}");
                let diag = schema_error(FILE, &problem, "give every actor a place and acts");
                diags.push(diag.at_path(format!("actors.{store}.{id}")));
            }
        }
    }
    for (id, place) in actors.place.iter() {
        // A walker stands on an exit, which must be one of the pack's.
        let known = match crate::state::exit_parts(place) {
            Some(_) => crate::rules::travel::exit_stood_on(canon, place).is_some(),
            None => canon.places.contains_key(place.as_str()),
        };
        if !known {
            let problem = format!("actor '{id}' is at '{place}', which the pack does not define");
            let diag = schema_error(FILE, &problem, "move the actor to a place");
            diags.push(diag.at_path(format!("actors.place.{id}")));
        }
    }
    for (key, def) in canon.meters.iter().chain(&canon.world_meters) {
        for carrier in find_lacking_carriers(state, &def.carrier, key) {
            let problem = format!("lacks meter '{key}', which the pack gives it");
            let hint = "run spook migrate to add the meter";
            diags.push(meter_error(&carrier, key, &problem, hint));
        }
    }
    for meter in find_stale_meters(state) {
        let problem = format!(
            "lists meter '{}', which the pack does not give it",
            meter.id
        );
        let hint = "run spook migrate to drop the meter";
        diags.push(meter_error(&meter.carrier, &meter.id, &problem, hint));
    }
    for (carrier, key, held) in held_meters(state) {
        let def = canon.meter_def(carrier.as_ref(), key);
        if def.is_some_and(|def| !meter::is_storable(def, held.base, held.offset)) {
            let problem = format!("meter '{key}' is outside the rules");
            let hint = "set base and offset so the stored pair fits the meter";
            diags.push(meter_error(&carrier, key, &problem, hint));
        }
    }
    for ((id, key), live) in find_stale_modifiers(state) {
        let problem = format!(
            "actor '{id}' holds modifier '{key}' on meter '{}', which the pack does not define",
            live.meter
        );
        let diag = schema_error(FILE, &problem, "run spook migrate to drop the modifier");
        diags.push(diag.at_path(format!("actors.modifiers.{id}.{key}")));
    }
    for ((id, item), count) in actors.items.iter() {
        if item.is_empty() {
            diags.push(
                schema_error(
                    FILE,
                    &format!("actor '{id}' lists an empty item id"),
                    "give the item a non-empty id",
                )
                .at_path(format!("actors.items.{id}")),
            );
        }
        if *count == 0 {
            diags.push(
                schema_error(
                    FILE,
                    &format!("actor '{id}' lists item '{item}' with a count of 0"),
                    "omit an item the actor does not hold",
                )
                .at_path(format!("actors.items.{id}.{item}")),
            );
        }
    }
    for ((place, item), count) in state.places().items.iter() {
        if *count == 0 {
            diags.push(
                schema_error(
                    FILE,
                    &format!("place '{place}' lists item '{item}' with a count of 0"),
                    "omit an item that is not there",
                )
                .at_path(format!("places.items.{place}.{item}")),
            );
        }
    }
    for (actor_id, item) in actors.items.keys() {
        if !canon.items.contains_key(item) {
            let path = format!("actors.items.{actor_id}.{item}");
            diags.push(unknown_item(FILE, item, &path));
        }
    }
    for (actor_id, wielded) in actors.wielded.iter() {
        if !canon.items.contains_key(wielded) {
            diags.push(unknown_item(
                FILE,
                wielded,
                &format!("actors.wielded.{actor_id}"),
            ));
        } else if !canon
            .items
            .get(wielded)
            .is_some_and(|item: &ItemDef| item.tags.iter().any(|tag| tag == "weapon"))
        {
            diags.push(
                schema_error(
                    FILE,
                    &format!("actor '{actor_id}' wields '{wielded}', which is not a weapon"),
                    "wield an item tagged weapon, or leave the slot empty",
                )
                .at_path(format!("actors.wielded.{actor_id}")),
            );
        } else if !actors
            .items
            .contains_key(&(actor_id.clone(), wielded.clone()))
        {
            diags.push(
                schema_error(
                    FILE,
                    &format!("actor '{actor_id}' wields '{wielded}' without holding it"),
                    "put the item in items or clear wielded",
                )
                .at_path(format!("actors.wielded.{actor_id}")),
            );
        }
    }
    for (place, item) in state.places().items.keys() {
        if !canon.items.contains_key(item) {
            let path = format!("places.items.{place}.{item}");
            diags.push(unknown_item(FILE, item, &path));
        }
    }
    if diags.is_empty() {
        Ok(())
    } else {
        Err(diags)
    }
}

/// A save error on meter `key` of `carrier`, at the meter's path: the carrier's name, then
/// `problem`.
fn meter_error(carrier: &entity::Id, key: &str, problem: &str, hint: &str) -> Diagnostic {
    let (name, path) = match carrier {
        entity::Id::Actor(id) => (format!("actor '{id}'"), format!("actors.meters.{id}")),
        entity::Id::Place(id) => (carrier.to_string(), format!("places.meters.{id}")),
        entity::Id::Story(id) => (carrier.to_string(), format!("stories.meters.{id}")),
        entity::Id::Global => ("the world".to_string(), "world.meters".to_string()),
    };
    let diag = schema_error(FILE, &format!("{name} {problem}"), hint);
    diag.at_path(format!("{path}.{key}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn world_stores_round_trip_byte_for_byte() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../packs/oneshot-docks/state/save.yaml");
        let empty = "world:
  meters: {}
  flags: []
";
        let held = "world:
  meters:
    dawn:
      base: 4
      offset: -1
    tide:
      base: 9
      offset: 0
  flags:
    - gate_locked
    - rang
";
        let original = fs::read_to_string(&path).unwrap();
        assert!(original.contains(empty));
        let original = original.replace(empty, held);
        let state = parse(&original).unwrap();
        assert_eq!(emit(&state).unwrap(), original);
    }

    #[test]
    fn docks_now_round_trips_through_emitter() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../packs/oneshot-docks/state/save.yaml");
        let original = fs::read_to_string(&path).unwrap();
        let state = parse(&original).unwrap();
        let emitted = emit(&state).unwrap();
        assert_eq!(emitted, original);
        assert_eq!(parse(&emitted).unwrap(), state);
    }

    fn save_text(tag: &str) -> String {
        format!(
            "\
schema: {tag}
world_id: ex
pack_rev: blake2s:00
rules_rev: {rules}
state_rev: 0
round_index: 0
world_time: 0
last_round_at: \"2026-01-01T00:00:00Z\"
actors: {{}}
notes: []
",
            rules = crate::store::RULES_REV,
        )
    }

    #[test]
    fn emit_writes_the_save_tag() {
        let mut state = parse(&save_text("save")).unwrap();
        state.schema = concat!("save/", "v0").to_string();
        let emitted = emit(&state).unwrap();
        assert!(emitted.starts_with("schema: save\n"), "{emitted}");
        assert_eq!(parse(&emitted).unwrap().schema, "save");
    }
}
