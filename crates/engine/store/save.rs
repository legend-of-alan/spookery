//! Read, write and migrate the save.

use serde::Deserialize;
use serde_saphyr::SerializerOptions;

use crate::canon::Canon;
use crate::diagnostic::{check_tag, error, locate_nth, parse_file, schema_error, Diagnostic};
use crate::model::item::holds_items;
use crate::model::meter::MeterLive;
use crate::model::modifier;
use crate::model::{exit_parts, sort_unique, Component, State};
use crate::rules::effect::Holder;
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
    let state: State = parse_file(FILE, text, 0)?;
    let mut diags = check_actor_places(&state);
    diags.extend(check_holders(&state));
    if diags.is_empty() {
        Ok(state)
    } else {
        Err(diags)
    }
}

pub fn emit(state: &State) -> Result<String, Reason> {
    let mut canonical = state.to_canonical();
    canonical.schema = crate::diagnostic::SAVE_SCHEMA.to_string();
    canonical.rules_rev = super::RULES_REV;
    let text = serde_saphyr::to_string_with_options(&canonical, emit_options())
        .map_err(|_| Reason::Schema)?;
    Ok(one_trailing_lf(text))
}

/// Each component's holder must be one the save lists: an actor in `actors`, a place in
/// `places`, a story in `stories`, an exit of a place in `places`, or the world. Only an actor
/// or a place holds items.
fn check_holders(state: &State) -> Vec<Diagnostic> {
    let wrong = state.components().filter(|(holder, component)| {
        let itemless = matches!(component, Component::Item(..)) && !holds_items(holder);
        !state.lists(holder) || itemless
    });
    let mut wrong: Vec<(&str, &Holder)> =
        (wrong.map(|(holder, component)| (component.store(), holder))).collect();
    wrong.dedup();
    let diag = |(store, holder): (&str, &Holder)| {
        let (problem, hint) = if state.lists(holder) {
            let problem = format!("{store} lists '{holder}', which cannot hold items");
            (problem, "give the items to an actor or a place")
        } else {
            let problem = format!("{store} lists '{holder}', which the save does not list");
            (problem, "add its record, or take its line out of the store")
        };
        schema_error("state/save.yaml", &problem, hint).at_path(format!("{store}.{holder}"))
    };
    wrong.into_iter().map(diag).collect()
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

fn check_actor_places(state: &State) -> Vec<Diagnostic> {
    const FILE: &str = "state/save.yaml";
    let mut diags = Vec::new();
    for (id, actor) in &state.actors {
        // A walker stands on an exit of a place until they arrive.
        let place = exit_parts(&actor.place).map_or(actor.place.as_str(), |(place, _)| place);
        if state.places.contains(place) {
            continue;
        }
        diags.push(
            schema_error(
                FILE,
                &format!(
                    "actor '{id}' is in place '{}', which is not in places",
                    actor.place
                ),
                "add the place or move the actor",
            )
            .at_path(format!("actors.{id}.place")),
        );
    }
    diags
}

/// Meters the pack gives a holder that the holder lacks, each at its `start`.
pub(crate) fn missing_meters(canon: &Canon, state: &State) -> Vec<AddedMeter> {
    let mut added = Vec::new();
    let lacks = |holder: &Holder, id: &str| state.meters.get(holder, id).is_none();
    for (id, def) in &canon.meters {
        let lacking = match &def.carrier {
            Some(holder) if !def.is_actor_meter() => lacks(holder, id),
            _ => state.actors.keys().any(|actor| {
                let actor = Holder::Actor(actor.clone());
                def.is_on(&actor) && lacks(&actor, id)
            }),
        };
        if lacking {
            added.push(AddedMeter::new(id, def));
        }
    }
    for (id, def) in &canon.world_meters {
        if lacks(&Holder::World, id) {
            added.push(AddedMeter::new(id, def));
        }
    }
    added
}

/// Meters an actor, place, story or the world lists that the pack no longer gives it.
pub(crate) fn stale_meters(canon: &Canon, state: &State) -> Vec<DroppedMeter> {
    let stale = state.meters.iter();
    let stale = stale.filter(|(carrier, id, _)| canon.meter_def(carrier, id).is_none());
    let dropped = stale.map(|(carrier, id, _)| DroppedMeter {
        id: id.clone(),
        carrier: carrier.clone(),
    });
    dropped.collect()
}

/// Modifiers some actor holds that the pack no longer defines, or now defines on another meter.
pub(crate) fn stale_modifiers(canon: &Canon, state: &State) -> Vec<String> {
    let mut stale: Vec<String> = state
        .modifiers
        .iter()
        .filter(|(_, id, live)| !modifier::is_defined(&canon.modifiers, id, live))
        .map(|(_, id, _)| id.clone())
        .collect();
    sort_unique(&mut stale);
    stale
}

/// Migrate a save: give each holder the `meters` it lacks, at their `start`, drop the
/// `dropped_modifiers` from every actor and each `dropped_meters` from its holder, then stamp
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
        let holders: Vec<Holder> = match &meter.carrier {
            Some(holder @ (Holder::Place(_) | Holder::Story(_))) => {
                state.make_record(holder)?;
                vec![holder.clone()]
            }
            Some(holder @ Holder::World) => vec![holder.clone()],
            carrier => (state.actors.keys())
                .map(|id| Holder::Actor(id.clone()))
                .filter(|actor| carrier.as_ref().is_none_or(|carrier| carrier == actor))
                .collect(),
        };
        for holder in holders {
            if state.meters.get(&holder, &meter.id).is_none() {
                state.put_meter(canon, &holder, &meter.id, Some(start.clone()))?;
            }
        }
    }
    let dropped: Vec<(String, String)> = (state.modifiers.iter())
        .filter(|(_, id, _)| dropped_modifiers.contains(id))
        .filter_map(|(holder, id, _)| match holder {
            Holder::Actor(actor) => Some((actor.clone(), id.clone())),
            _ => None,
        })
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
places:
  - gate
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
