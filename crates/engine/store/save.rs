//! Read, write and migrate the save.

use serde::Deserialize;
use serde_saphyr::SerializerOptions;

use crate::canon::Canon;
use crate::diagnostic::{check_tag, error, locate_nth, parse_file, Diagnostic};
use crate::model::meter::MeterLive;
use crate::model::modifier;
use crate::model::{sort_unique, State};
use crate::rules::effect::EntityId;
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
    let mut canonical = state.to_canonical();
    canonical.schema = crate::diagnostic::SAVE_SCHEMA.to_string();
    canonical.rules_rev = super::RULES_REV;
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

/// Meters the pack gives an entity that the entity lacks, each at its `start`.
pub(crate) fn missing_meters(canon: &Canon, state: &State) -> Vec<AddedMeter> {
    let mut added = Vec::new();
    let records = state.actors().records();
    let lacks = |entity: &EntityId, id: &str| state.meters_of(entity).get(id).is_none();
    for (id, def) in &canon.meters {
        let lacking = match &def.carrier {
            None => (records.keys()).any(|actor| lacks(&EntityId::Actor(actor.clone()), id)),
            Some(actor @ EntityId::Actor(actor_id)) => {
                records.contains_key(actor_id) && lacks(actor, id)
            }
            Some(entity @ (EntityId::Place(_) | EntityId::Story(_) | EntityId::World)) => {
                lacks(entity, id)
            }
        };
        if lacking {
            added.push(AddedMeter::new(id, def));
        }
    }
    for (id, def) in &canon.world_meters {
        if lacks(&EntityId::World, id) {
            added.push(AddedMeter::new(id, def));
        }
    }
    added
}

/// Meters an actor, place, story or the world lists that the pack no longer gives it.
pub(crate) fn stale_meters(canon: &Canon, state: &State) -> Vec<DroppedMeter> {
    let actors = state.actors().meters().iter();
    let actors = actors.map(|(id, key, _)| (EntityId::Actor(id.clone()), key));
    let places = state.places().meters().iter();
    let places = places.map(|(id, key, _)| (EntityId::Place(id.clone()), key));
    let stories = state.stories().meters().iter();
    let stories = stories.map(|(id, key, _)| (EntityId::Story(id.clone()), key));
    let world = (state.world().meters().keys()).map(|key| (EntityId::World, key));
    let held = actors.chain(places).chain(stories).chain(world);
    let stale = held.filter(|(carrier, id)| canon.meter_def(carrier, id).is_none());
    let dropped = stale.map(|(carrier, id)| DroppedMeter {
        id: id.clone(),
        carrier,
    });
    dropped.collect()
}

/// Modifiers some actor holds that the pack no longer defines, or now defines on another meter.
pub(crate) fn stale_modifiers(canon: &Canon, state: &State) -> Vec<String> {
    let mut stale: Vec<String> = (state.actors().modifiers().iter())
        .filter(|(_, id, live)| !modifier::is_defined(&canon.modifiers, id, live))
        .map(|(_, id, _)| id.clone())
        .collect();
    sort_unique(&mut stale);
    stale
}

/// Migrate a save: give each entity the `meters` it lacks, at their `start`, drop the
/// `dropped_modifiers` from every actor and each `dropped_meters` from its entity, then stamp
/// `pack_rev` and `state_rev`. Live migrate and replay both call this. It does not touch the disk.
pub fn migrate(
    canon: &Canon,
    mut state: State,
    pack_rev: &str,
    state_rev: u64,
    meters: &[AddedMeter],
    dropped_modifiers: &[String],
    dropped_meters: &[DroppedMeter],
) -> Result<State, Reason> {
    for meter in meters {
        let start = MeterLive {
            base: meter.start,
            offset: 0,
        };
        let records = state.actors().records();
        let entities: Vec<EntityId> = match &meter.carrier {
            None => (records.keys())
                .map(|id| EntityId::Actor(id.clone()))
                .collect(),
            Some(actor @ EntityId::Actor(id)) => {
                (records.contains_key(id).then(|| actor.clone()).into_iter()).collect()
            }
            Some(entity @ (EntityId::Place(_) | EntityId::Story(_) | EntityId::World)) => {
                vec![entity.clone()]
            }
        };
        for entity in entities {
            if state.meters_of(&entity).get(&meter.id).is_none() {
                state.put_meter(canon, &entity, &meter.id, Some(start.clone()))?;
            }
        }
    }
    let dropped: Vec<(String, String)> = (state.actors().modifiers().iter())
        .filter(|(_, id, _)| dropped_modifiers.contains(id))
        .map(|(actor, id, _)| (actor.clone(), id.clone()))
        .collect();
    for (actor, id) in dropped {
        state.put_modifier(canon, &actor, &id, None)?;
    }
    for meter in dropped_meters {
        state.put_meter(canon, &meter.carrier, &meter.id, None)?;
    }
    state.pack_rev = pack_rev.to_string();
    state.state_rev = state_rev;
    state.accept();
    Ok(state)
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
