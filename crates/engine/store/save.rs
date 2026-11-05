//! Read, write and migrate the save.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_saphyr::SerializerOptions;

use crate::canon::Canon;
use crate::diagnostic::{check_tag, error, locate_nth, parse_file, schema_error, Diagnostic};
use crate::model::meter::MeterLive;
use crate::model::modifier;
use crate::model::{sort_unique, State};
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
    let diags = check_actor_places(&state);
    if diags.is_empty() {
        Ok(state)
    } else {
        Err(diags)
    }
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
    for actor in state.actors.values_mut() {
        sort_unique(&mut actor.flags);
        sort_unique(&mut actor.known_facts);
    }
    sort_unique(&mut state.flags);
}

/// A clock as a save from before meters held it: its value and who owned it.
#[derive(Deserialize)]
pub(crate) struct OldClock {
    value: i64,
    owners: Vec<String>,
}

/// The last rules version whose saves held clocks.
const CLOCK_RULES_REV: u64 = 11;

/// The clocks of a save from before meters, by id. `None` for a save in the current shape.
pub(crate) type OldClocks = Option<BTreeMap<String, OldClock>>;

/// Parse a save to migrate. A save from the rules before meters replaced clocks is read too:
/// its `pause` is dropped and its clocks come back beside the state, for `carry_clock`.
pub(crate) fn parse_for_migrate(text: &str) -> Result<(State, OldClocks), Vec<Diagnostic>> {
    const FILE: &str = "state/save.yaml";
    #[derive(Deserialize)]
    struct OldSave {
        rules_rev: u64,
        clocks: BTreeMap<String, OldClock>,
    }
    let old = serde_saphyr::from_str::<OldSave>(text)
        .ok()
        .filter(|old| old.rules_rev == CLOCK_RULES_REV);
    let Some(old) = old else {
        return parse(text).map(|state| (state, None));
    };
    let unreadable = || {
        vec![schema_error(
            FILE,
            "the save does not read",
            "start a fresh save",
        )]
    };
    let mut fields: serde_json::Map<String, serde_json::Value> =
        serde_saphyr::from_str(text).map_err(|_| unreadable())?;
    fields.remove("clocks");
    fields.remove("pause");
    fields.insert("rules_rev".into(), super::RULES_REV.into());
    let state: State = serde_json::from_value(fields.into()).map_err(|_| unreadable())?;
    let diags = check_actor_places(&state);
    if !diags.is_empty() {
        return Err(diags);
    }
    Ok((state, Some(old.clocks)))
}

/// Move clock `id`'s value onto the meter that replaced it: the world's when no one owned the
/// clock, else its one owner's. A clock with more owners has no one meter to go to.
pub(crate) fn carry_clock(
    canon: &Canon,
    state: &mut State,
    id: &str,
    clock: OldClock,
) -> Result<(), Diagnostic> {
    let path = format!("clocks.{id}");
    let (holder, file) = match clock.owners.as_slice() {
        [] => (Holder::World, "world/meters.yaml".to_string()),
        [owner] => (
            Holder::Actor(owner.clone()),
            format!("the cast file of {owner}"),
        ),
        owners => {
            let problem = format!(
                "clock '{id}' has {} owners, and a meter has one holder",
                owners.len()
            );
            let hint = "give the clock one owner, or none for a world meter, then migrate again";
            return Err(schema_error("state/save.yaml", &problem, hint).at_path(path));
        }
    };
    let Some(def) = canon.meter_def(&holder, id) else {
        let problem = format!("clock '{id}' has no meter to move to");
        let hint = format!("declare a meter {id} in {file}, then migrate again");
        return Err(schema_error("state/save.yaml", &problem, &hint).at_path(path));
    };
    let live = MeterLive {
        base: def.start,
        offset: clock.value - def.start,
    };
    let Ok(meters) = state.meters_mut(&holder) else {
        let owner = match &holder {
            Holder::Actor(owner) => owner.as_str(),
            _ => "world",
        };
        let problem = format!("clock '{id}' is owned by '{owner}', and the save has no such actor");
        let hint = "give the clock an owner the save has, then migrate again";
        return Err(schema_error("state/save.yaml", &problem, hint).at_path(path));
    };
    meters.insert(id.to_string(), live);
    Ok(())
}

fn check_actor_places(state: &State) -> Vec<Diagnostic> {
    const FILE: &str = "state/save.yaml";
    let mut diags = Vec::new();
    for (id, actor) in &state.actors {
        if state.places.contains_key(&actor.place) {
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
    for (id, def) in &canon.meters {
        let lacking = state
            .actors
            .iter()
            .any(|(actor, live)| def.is_on_actor(actor) && !live.meters.contains_key(id));
        if lacking {
            added.push(AddedMeter::new(id, def));
        }
    }
    for (id, def) in &canon.world_meters {
        if !state.world_meters.contains_key(id) {
            added.push(AddedMeter::new(id, def));
        }
    }
    added
}

/// Meters an actor or the world lists that the pack no longer gives it. A place holds no meters.
pub(crate) fn stale_meters(canon: &Canon, state: &State) -> Vec<DroppedMeter> {
    let mut dropped = Vec::new();
    let mut drop_stale = |carrier: Holder, live: &BTreeMap<String, MeterLive>| {
        for id in live.keys() {
            if canon.meter_def(&carrier, id).is_none() {
                dropped.push(DroppedMeter {
                    id: id.clone(),
                    carrier: carrier.clone(),
                });
            }
        }
    };
    for (id, actor) in &state.actors {
        drop_stale(Holder::Actor(id.clone()), &actor.meters);
    }
    drop_stale(Holder::World, &state.world_meters);
    dropped
}

/// Modifiers some actor holds that the pack no longer defines, or now defines on another meter.
pub(crate) fn stale_modifiers(canon: &Canon, state: &State) -> Vec<String> {
    let mut stale: Vec<String> = state
        .actors
        .values()
        .flat_map(|actor| &actor.modifiers)
        .filter(|(id, live)| !modifier::is_defined(&canon.modifiers, id, live))
        .map(|(id, _)| id.clone())
        .collect();
    sort_unique(&mut stale);
    stale
}

/// Migrate a save: give each holder the `meters` it lacks, at their `start`, drop the
/// `dropped_modifiers` from every actor and each `dropped_meters` from its holder, then stamp
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
        let start = MeterLive {
            base: meter.start,
            offset: 0,
        };
        match &meter.carrier {
            Some(Holder::World) => {
                let live = state.world_meters.entry(meter.id.clone());
                live.or_insert(start);
            }
            Some(Holder::Place(_)) => {}
            carrier => {
                for (id, actor) in &mut state.actors {
                    if carrier
                        .as_ref()
                        .is_none_or(|carrier| *carrier == Holder::Actor(id.clone()))
                    {
                        let live = actor.meters.entry(meter.id.clone());
                        live.or_insert(start.clone());
                    }
                }
            }
        }
    }
    for actor in state.actors.values_mut() {
        actor
            .modifiers
            .retain(|id, _| !dropped_modifiers.contains(id));
    }
    for meter in dropped_meters {
        if let Ok(live) = state.meters_mut(&meter.carrier) {
            live.remove(&meter.id);
        }
    }
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
places:
  gate:
    items: {{}}
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
