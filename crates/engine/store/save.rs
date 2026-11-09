//! Read, write and migrate the save.

use serde::Deserialize;
use serde_saphyr::SerializerOptions;

use crate::canon::Canon;
use crate::diagnostic::{check_tag, error, locate_nth, parse_file, Diagnostic};
use crate::state::id::{ActorId, EntityId};
use crate::state::meter::Meter;
use crate::state::modifier;
use crate::state::{nested_map, sort_unique, State};
use crate::store::records::{AddedMeter, DroppedMeter};
use crate::Reason;

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

pub fn parse(text: &str) -> Result<State, Vec<Diagnostic>> {
    const FILE: &str = "state/save.yaml";
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

pub fn emit(state: &State) -> Result<String, Reason> {
    let mut canonical = state.clone();
    canonical.schema = crate::diagnostic::SAVE_SCHEMA.to_string();
    canonical.rules_rev = super::RULES_REV;
    normalize(&mut canonical);
    let text = serde_saphyr::to_string_with_options(&canonical, emit_options())
        .map_err(|_| Reason::Schema)?;
    Ok(one_trailing_lf(text))
}

fn rules_rev_diagnostic(text: &str) -> Diagnostic {
    let mut diag = error(
        "state/save.yaml",
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

pub fn normalize(state: &mut State) {
    state.notes.sort_by(|a, b| a.id.cmp(&b.id));
    for note in &mut state.notes {
        sort_unique(&mut note.to);
        sort_unique(&mut note.fact_ids);
    }
    for known in state.actors.known_facts.values_mut() {
        sort_unique(known);
    }
}

/// Meters the pack gives an entity that the entity lacks, each at its `start`.
pub(crate) fn missing_meters(canon: &Canon, state: &State) -> Vec<AddedMeter> {
    let mut added = Vec::new();
    for (id, def) in &canon.meters {
        if !find_lacking_carriers(state, &def.carrier, id).is_empty() {
            added.push(AddedMeter::new(id, def));
        }
    }
    for (id, def) in &canon.world_meters {
        if !state.world.meters.contains_key(id) {
            added.push(AddedMeter::new(id, def));
        }
    }
    added
}

/// The entities `carrier` names that lack meter `id`. Every-actor and one-actor carriers are read
/// off the save's actors, so a cast member the save does not list names nobody; any other carrier
/// is itself. The one kind match migrate keeps of its own.
pub(crate) fn find_lacking_carriers(
    state: &State,
    carrier: &Option<EntityId>,
    id: &str,
) -> Vec<EntityId> {
    let held = match carrier {
        None | Some(EntityId::Actor(_)) => {
            let actors = state.actors();
            let named =
                |actor: &&ActorId| !matches!(carrier, Some(EntityId::Actor(one)) if one != *actor);
            let lacks = |actor: &&ActorId| actors.meter(actor, id).is_none();
            let lacking = actors.kind.keys().filter(named).filter(lacks).cloned();
            return lacking.map(EntityId::Actor).collect();
        }
        Some(EntityId::Place(place)) => nested_map::get(&state.places().meters, place, id),
        Some(EntityId::Story(story)) => nested_map::get(&state.stories().meters, story, id),
        Some(EntityId::World) => state.world().meters.get(id),
    };
    held.is_none()
        .then(|| carrier.clone())
        .flatten()
        .into_iter()
        .collect()
}

/// Meters an actor, place, story or the world lists that the pack no longer gives it.
pub(crate) fn stale_meters(canon: &Canon, state: &State) -> Vec<DroppedMeter> {
    let actors = state.actors.meters.keys();
    let places = state.places.meters.keys();
    let stories = state.stories.meters.keys();
    let carriers = (actors.map(|(id, key)| (EntityId::Actor(id.clone()), key)))
        .chain(places.map(|(id, key)| (EntityId::Place(id.clone()), key)))
        .chain(stories.map(|(id, key)| (EntityId::Story(id.clone()), key)))
        .chain(state.world.meters.keys().map(|key| (EntityId::World, key)));
    carriers
        .filter(|(carrier, id)| canon.meter_def(carrier.as_ref(), id).is_none())
        .map(|(carrier, id)| DroppedMeter {
            id: id.clone(),
            carrier,
        })
        .collect()
}

/// Modifiers some actor holds that the pack no longer defines, or now defines on another meter.
pub(crate) fn stale_modifiers(canon: &Canon, state: &State) -> Vec<String> {
    let mut stale: Vec<String> = state
        .actors
        .modifiers
        .iter()
        .filter(|((_, id), live)| !modifier::is_defined(&canon.modifiers, id, live))
        .map(|((_, id), _)| id.clone())
        .collect();
    sort_unique(&mut stale);
    stale
}

/// Migrate a save: give each entity the `meters` it lacks, at their `start`, drop the
/// `dropped_modifiers` from every actor and each `dropped_meters` from its entity, then stamp
/// `pack_rev` and `state_rev`. Live migrate and replay both call this. It does not touch the disk.
pub fn migrate(
    mut state: State,
    pack_rev: &str,
    state_rev: u64,
    meters: &[AddedMeter],
    dropped_modifiers: &[String],
    dropped_meters: &[DroppedMeter],
) -> State {
    for meter in meters {
        let start = Meter {
            base: meter.start,
            offset: 0,
        };
        let key = meter.id.clone();
        match &meter.carrier {
            Some(EntityId::World) => {
                state.world.meters.entry(key).or_insert(start);
            }
            Some(EntityId::Place(id)) => {
                state
                    .places
                    .meters
                    .entry((id.clone(), key))
                    .or_insert(start);
            }
            Some(EntityId::Story(id)) => {
                state
                    .stories
                    .meters
                    .entry((id.clone(), key))
                    .or_insert(start);
            }
            carrier => {
                for id in state.actors.kind.keys() {
                    if carrier
                        .as_ref()
                        .is_none_or(|carrier| *carrier == EntityId::Actor(id.clone()))
                    {
                        let live = state.actors.meters.entry((id.clone(), key.clone()));
                        live.or_insert(start.clone());
                    }
                }
            }
        }
    }
    let dropped = |carrier: EntityId, id: &String| {
        dropped_meters.contains(&DroppedMeter {
            id: id.clone(),
            carrier,
        })
    };
    state
        .actors
        .modifiers
        .retain(|(_, id), _| !dropped_modifiers.contains(id));
    (state.actors.meters).retain(|(on, id), _| !dropped(EntityId::Actor(on.clone()), id));
    (state.places.meters).retain(|(on, id), _| !dropped(EntityId::Place(on.clone()), id));
    (state.stories.meters).retain(|(on, id), _| !dropped(EntityId::Story(on.clone()), id));
    (state.world.meters).retain(|id, _| !dropped(EntityId::World, id));
    state.pack_rev = pack_rev.to_string();
    state.state_rev = state_rev;
    normalize(&mut state);
    state
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
