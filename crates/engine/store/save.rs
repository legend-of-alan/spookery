//! Read, write, migrate and check the save.

use serde::Deserialize;
use serde_saphyr::SerializerOptions;

use crate::canon::{unknown_item, ItemDef};
use crate::diagnostic::{check_tag, error, locate_nth, parse_file, schema_error, Diagnostic};
use crate::entity;
use crate::state::component::meter::{self, Meter};
use crate::state::component::modifier::{self, Modifier};
use crate::state::id::{ActorId, GlobalId};
use crate::state::{component, Save, State};
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
    state: &State,
    carrier: &Option<entity::Id>,
    id: &str,
) -> Vec<entity::Id> {
    let held = match carrier {
        None | Some(entity::Id::Actor(_)) => {
            let actors = state.actors();
            let named = |actor: &&ActorId| match carrier {
                Some(entity::Id::Actor(one)) => one == *actor,
                _ => true,
            };
            let lacks = |actor: &&ActorId| !actors.get::<Meter>(actor).contains(id);
            let ids = actors.query::<ActorId>();
            let lacking = ids.iter().filter(named).filter(lacks).cloned();
            return lacking.map(entity::Id::Actor).collect();
        }
        Some(entity::Id::Place(place)) => state.places().get::<Meter>(place).get(id),
        Some(entity::Id::Story(story)) => state.stories().get::<Meter>(story).get(id),
        Some(entity::Id::Global) => state.globals().get::<Meter>(&GlobalId).get(id),
    };
    held.is_none()
        .then(|| carrier.clone())
        .flatten()
        .into_iter()
        .collect()
}

/// Every meter the save lists, with the actor, place, story or world that holds it.
fn held_meters(state: &State) -> impl Iterator<Item = (entity::Id, &str, &Meter)> {
    let actors = state.actors().query::<&Meter>();
    let places = state.places().query::<&Meter>();
    let stories = state.stories().query::<&Meter>();
    let world = state.globals().query::<&Meter>();
    let actors = (actors.iter().flat_map(|held| held.iter_with_id()))
        .map(|(id, key, held)| (entity::Id::Actor(id.clone()), key, held));
    let places = (places.iter().flat_map(|held| held.iter_with_id()))
        .map(|(id, key, held)| (entity::Id::Place(id.clone()), key, held));
    let stories = (stories.iter().flat_map(|held| held.iter_with_id()))
        .map(|(id, key, held)| (entity::Id::Story(id.clone()), key, held));
    let world = (world.iter().flat_map(|held| held.iter_with_id()))
        .map(|(_, key, held)| (entity::Id::Global, key, held));
    actors.chain(places).chain(stories).chain(world)
}

/// Meters an actor, place, story or the world lists that the pack no longer gives it.
pub(crate) fn find_stale_meters(state: &State) -> Vec<DroppedMeter> {
    held_meters(state)
        .filter(|(carrier, id, _)| state.canon().meter_def(carrier.as_ref(), id).is_none())
        .map(|(carrier, id, _)| DroppedMeter {
            id: id.to_string(),
            carrier,
        })
        .collect()
}

/// Each actor's modifiers that the pack no longer defines, or now defines on another meter:
/// the actor, the modifier id and the modifier as the save holds it.
pub(crate) fn find_stale_modifiers(state: &State) -> Vec<(&ActorId, &str, &Modifier)> {
    let actors = state.actors().query::<&Modifier>();
    let held = actors.iter().flat_map(|held| held.iter_with_id());
    held.filter(|(_, id, live)| !modifier::is_defined(&state.canon().modifiers, id, live))
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
            (state.entities_mut().get_mut::<Meter>(&id)).set(&meter.id, Some(start.clone()))?;
        }
    }
    let actors = state.actors().query::<&Modifier>();
    let held = actors.iter().flat_map(|held| held.iter_with_id());
    let dropped = held.filter(|(_, id, _)| record.dropped_modifiers.iter().any(|drop| drop == id));
    let held: Vec<(ActorId, String)> =
        (dropped.map(|(actor, id, _)| (actor.clone(), id.to_string()))).collect();
    for (actor, id) in &held {
        (state.actors_mut().get_mut::<Modifier>(actor)).set(id, None)?;
    }
    for meter in &record.dropped_meters {
        (state.entities_mut().get_mut::<Meter>(&meter.carrier)).set(&meter.id, None)?;
    }
    state.pack_rev = record.pack_rev_after.clone();
    state.state_rev = record.state_rev_after;
    state.accept();
    Ok(state)
}

/// Check the save against the pack without changing it. Refuse every component on an actor with no
/// control type, every actor without a place or an action state, every actor in a place or on an
/// exit the pack lacks, every meter, item and wielded slot the pack does not define, every meter
/// the pack gives an entity that the entity lacks, every meter outside its rules, every modifier
/// the pack no longer gives that meter, and every empty id or zero count.
pub fn check_save(state: &State) -> Result<(), Vec<Diagnostic>> {
    let canon = state.canon();
    let mut diags = Vec::new();
    let actors = state.actors();
    let held = actors.carriers_by_store();
    let lacking = held.iter().filter(|(_, id)| !actors.contains(id));
    for (store, id) in lacking {
        let problem = format!("actor '{id}' has {store} but no control type");
        let hint = "give the actor a control type under actors.control_type";
        let diag = schema_error(FILE, &problem, hint);
        diags.push(diag.at_path(format!("actors.{store}.{id}")));
    }
    for id in actors.query::<ActorId>().iter() {
        for store in ["place", "acts"] {
            if !held.contains(&(store, id)) {
                let problem = format!("actor '{id}' has a control type but no {store}");
                let diag = schema_error(FILE, &problem, "give every actor a place and acts");
                diags.push(diag.at_path(format!("actors.{store}.{id}")));
            }
        }
    }
    for (id, place) in actors.query::<(ActorId, &component::Place)>().iter() {
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
    for (id, key, live) in find_stale_modifiers(state) {
        let problem = format!(
            "actor '{id}' holds modifier '{key}' on meter '{}', which the pack does not define",
            live.meter
        );
        let diag = schema_error(FILE, &problem, "run spook migrate to drop the modifier");
        diags.push(diag.at_path(format!("actors.modifiers.{id}.{key}")));
    }
    let held = actors.query::<&component::Item>();
    let actor_items: Vec<_> = held.iter().flat_map(|held| held.iter_with_id()).collect();
    for &(id, item, count) in &actor_items {
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
    let held = state.places().query::<&component::Item>();
    let place_items: Vec<_> = held.iter().flat_map(|held| held.iter_with_id()).collect();
    for &(place, item, count) in &place_items {
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
    for (actor_id, item, _) in actor_items {
        if !canon.items.contains_key(item) {
            let path = format!("actors.items.{actor_id}.{item}");
            diags.push(unknown_item(FILE, item, &path));
        }
    }
    for (actor_id, wielded) in actors.query::<(ActorId, &component::WieldedItem)>().iter() {
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
        } else if !actors.get::<component::Item>(actor_id).contains(wielded) {
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
    for (place, item, _) in place_items {
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
    fn docks_now_round_trips_through_emitter() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../packs/oneshot-docks/state/save.yaml");
        let original = fs::read_to_string(&path).unwrap();
        // The same save with a world that holds meters and flags, which key on `GlobalId`.
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
        assert!(original.contains(empty));
        for text in [original.clone(), original.replace(empty, held)] {
            let state = parse(&text).unwrap();
            let emitted = emit(&state).unwrap();
            assert_eq!(emitted, text);
            assert_eq!(parse(&emitted).unwrap(), state);
        }
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
