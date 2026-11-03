//! Read, write and migrate the save.

use serde::Deserialize;
use serde_saphyr::SerializerOptions;

use crate::canon::Canon;
use crate::diagnostic::{check_tag, error, locate_nth, parse_file, schema_error, Diagnostic};
use crate::model::meter::MeterLive;
use crate::model::{sort_unique, State};
use crate::store::records::AddedMeter;
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
    for clock in state.clocks.values_mut() {
        sort_unique(&mut clock.owners);
    }
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

/// Meters the pack defines that at least one actor lacks, each at its `start`.
pub(crate) fn missing_meters(canon: &Canon, state: &State) -> Vec<AddedMeter> {
    canon
        .meters
        .iter()
        .filter(|(id, _)| {
            state
                .actors
                .values()
                .any(|actor| !actor.meters.contains_key(*id))
        })
        .map(|(id, def)| AddedMeter {
            id: id.clone(),
            start: def.start,
        })
        .collect()
}

/// Migrate a save: give each actor the `meters` it lacks, at their `start`, then
/// stamp `pack_rev` and `state_rev`. Live migrate and replay both call this.
/// It does not touch the disk.
pub fn migrate(mut state: State, pack_rev: &str, state_rev: u64, meters: &[AddedMeter]) -> State {
    for actor in state.actors.values_mut() {
        let live = &mut actor.meters;
        for meter in meters {
            live.entry(meter.id.clone()).or_insert(MeterLive {
                base: meter.start,
                offset: 0,
            });
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

    fn parse_effect(text: &str) -> Result<Effect, Reason> {
        let record: crate::rules::effect::EffectRecord =
            serde_json::from_str(text).map_err(|_| Reason::Schema)?;
        Effect::try_from(record)
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

    #[test]
    fn a_save_that_still_lists_place_occupants_is_refused() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../packs/oneshot-docks/state/save.yaml");
        let current = fs::read_to_string(&path).unwrap();
        let place = "  mira_rooms:\n    items: {}\n";
        let listed = current.replacen(
            place,
            &format!("{place}    notes: []\n    actors:\n      - mira\n"),
            1,
        );
        assert_ne!(listed, current);
        let refused = parse(&listed).unwrap_err();
        assert!(
            refused[0].message.starts_with("unknown field 'notes'"),
            "{refused:?}"
        );
        assert_eq!(crate::diagnostic::first_reason(&refused), Reason::Schema);
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
pause:
  presence_clocks: paused
places:
  gate:
    items: {{}}
actors: {{}}
clocks: {{}}
notes: []
",
            rules = crate::store::RULES_REV,
        )
    }

    #[test]
    fn an_unknown_tag_and_the_old_versioned_tag_are_schema() {
        assert!(parse(&save_text("save")).is_ok());
        let old = parse(&save_text(concat!("save/", "v0"))).unwrap_err();
        assert!(old[0].message.contains("no version"), "{old:?}");
        assert_eq!(crate::diagnostic::first_reason(&old), Reason::Schema);
        let bogus = parse(&save_text("bogus")).unwrap_err();
        assert!(bogus[0].message.contains("does not use"), "{bogus:?}");
        assert_eq!(crate::diagnostic::first_reason(&bogus), Reason::Schema);
    }

    #[test]
    fn emit_writes_the_save_tag() {
        let mut state = parse(&save_text("save")).unwrap();
        state.schema = concat!("save/", "v0").to_string();
        let emitted = emit(&state).unwrap();
        assert!(emitted.starts_with("schema: save\n"), "{emitted}");
        assert_eq!(parse(&emitted).unwrap().schema, "save");
    }

    use crate::intent::Viewer;
    use crate::model::MemoryLine;
    use crate::rules::effect::{Effect, EffectKind};
    use crate::store::records::emit_effect;
    use crate::Reason;

    fn effect(kind: EffectKind, viewer: Viewer, intent_id: &str) -> Effect {
        Effect {
            kind,
            viewer,
            intent_id: intent_id.to_string(),
        }
    }

    fn npc(id: &str) -> Viewer {
        Viewer::Npc(id.to_string())
    }

    fn player(id: &str) -> Viewer {
        Viewer::Player(id.to_string())
    }

    #[test]
    fn effect_json_key_order_is_kind_as_intent_then_payload_fields() {
        let cases = [
            (
                effect(
                    EffectKind::Clock {
                        clock_id: "shop_day".to_string(),
                        delta: 1,
                        unpause: false,
                    },
                    npc("mira"),
                    "i1",
                ),
                r#"{"kind":"clock","as":"npc:mira","intent_id":"i1","clock_id":"shop_day","delta":1}"#,
            ),
            (
                effect(
                    EffectKind::Meter {
                        actor_id: "mira".to_string(),
                        key: "rest".to_string(),
                        write: crate::model::meter::MeterWrite::Offset(1),
                    },
                    npc("mira"),
                    "i2",
                ),
                r#"{"kind":"meter","as":"npc:mira","intent_id":"i2","actor_id":"mira","key":"rest","write":"offset","delta":1}"#,
            ),
            (
                effect(
                    EffectKind::Item {
                        actor_id: None,
                        place: Some("docks_shop".to_string()),
                        id: "lamp_oil".to_string(),
                        delta: 4,
                        max: Some(8),
                    },
                    npc("mira"),
                    "i3",
                ),
                r#"{"kind":"item","as":"npc:mira","intent_id":"i3","place":"docks_shop","id":"lamp_oil","delta":4,"max":8}"#,
            ),
            (
                effect(
                    EffectKind::Location {
                        actor_id: "bram".to_string(),
                        from: "docks_street".to_string(),
                        to: "docks_shop".to_string(),
                        arrived_at: None,
                    },
                    npc("bram"),
                    "i4",
                ),
                r#"{"kind":"location","as":"npc:bram","intent_id":"i4","actor_id":"bram","from":"docks_street","to":"docks_shop"}"#,
            ),
            (
                effect(
                    EffectKind::Memory {
                        actor_id: "mira".to_string(),
                        fact_ids: vec!["fact_a".to_string(), "fact_b".to_string()],
                        tags: vec!["heard".to_string()],
                        text: Some("oil".to_string()),
                    },
                    npc("mira"),
                    "i7",
                ),
                r#"{"kind":"memory","as":"npc:mira","intent_id":"i7","actor_id":"mira","fact_ids":["fact_a","fact_b"],"tags":["heard"],"text":"oil"}"#,
            ),
            (
                effect(
                    EffectKind::Memory {
                        actor_id: "mira".to_string(),
                        fact_ids: Vec::new(),
                        tags: Vec::new(),
                        text: None,
                    },
                    npc("mira"),
                    "i7b",
                ),
                r#"{"kind":"memory","as":"npc:mira","intent_id":"i7b","actor_id":"mira","fact_ids":[],"tags":[]}"#,
            ),
            (
                effect(
                    EffectKind::Meter {
                        actor_id: "pc_rowan".to_string(),
                        key: "grit".to_string(),
                        write: crate::model::meter::MeterWrite::Offset(-3),
                    },
                    player("pc_rowan"),
                    "i9",
                ),
                r#"{"kind":"meter","as":"player:pc_rowan","intent_id":"i9","actor_id":"pc_rowan","key":"grit","write":"offset","delta":-3}"#,
            ),
            (
                effect(
                    EffectKind::Flag {
                        actor_id: "mira".to_string(),
                        key: "marked".to_string(),
                        value: true,
                    },
                    npc("mira"),
                    "i10",
                ),
                r#"{"kind":"flag","as":"npc:mira","intent_id":"i10","actor_id":"mira","key":"marked","value":true}"#,
            ),
            (
                effect(
                    EffectKind::Note {
                        note_id: "note_mira_door".to_string(),
                        to: vec!["pc_rowan".to_string()],
                        place: Some("docks_shop".to_string()),
                        fact_ids: Vec::new(),
                        text: "say \"hi\"".to_string(),
                    },
                    npc("mira"),
                    "i12",
                ),
                r#"{"kind":"note","as":"npc:mira","intent_id":"i12","note_id":"note_mira_door","to":["pc_rowan"],"place":"docks_shop","fact_ids":[],"text":"say \"hi\""}"#,
            ),
            (
                effect(
                    EffectKind::Note {
                        note_id: "n".to_string(),
                        to: Vec::new(),
                        place: None,
                        fact_ids: vec!["f".to_string()],
                        text: "x".to_string(),
                    },
                    npc("mira"),
                    "i12b",
                ),
                r#"{"kind":"note","as":"npc:mira","intent_id":"i12b","note_id":"n","to":[],"fact_ids":["f"],"text":"x"}"#,
            ),
            (
                effect(
                    EffectKind::Reveal {
                        actor_id: "pc_nell".to_string(),
                        fact: "fact_trapdoor".to_string(),
                    },
                    player("pc_nell"),
                    "i13",
                ),
                r#"{"kind":"reveal","as":"player:pc_nell","intent_id":"i13","actor_id":"pc_nell","fact":"fact_trapdoor"}"#,
            ),
        ];
        for (effect, expected) in cases {
            let encoded = emit_effect(&effect);
            assert_eq!(encoded, expected);
            assert!(!encoded.contains("\"payload\""), "{encoded}");
            assert_eq!(parse_effect(&encoded).unwrap(), effect);
        }
        for name in ["need", "resource", "inventory"] {
            let value = serde_json::json!({
                "kind": name,
                "as": "npc:mira",
                "intent_id": "i",
            });
            assert_eq!(
                parse_effect(&value.to_string()).unwrap_err(),
                Reason::UnknownEffect
            );
        }
        let missing_delta = serde_json::json!({
            "kind": "meter",
            "as": "npc:mira",
            "intent_id": "i",
            "actor_id": "mira",
            "key": "rest",
            "write": "offset",
        });
        assert_eq!(
            parse_effect(&missing_delta.to_string()).unwrap_err(),
            Reason::Schema
        );
        let mut unknown_key = missing_delta;
        unknown_key["delta"] = 1.into();
        unknown_key["max"] = 3.into();
        assert_eq!(
            parse_effect(&unknown_key.to_string()).unwrap_err(),
            Reason::Schema
        );
    }

    #[test]
    fn effect_payload_object_keeps_declaration_order_and_a_negative_delta() {
        let effect = parse_effect(
            r#"{"kind":"item","as":"npc:n","intent_id":"i","place":"p","id":"s","delta":-1}"#,
        )
        .unwrap();
        assert_eq!(
            crate::rules::effect::payload_json(&effect),
            r#"{"place":"p","id":"s","delta":-1}"#
        );
    }

    #[test]
    fn nested_payload_and_external_tag_are_not_effects() {
        let nested = r#"{"kind":"clock","as":"npc:mira","intent_id":"i","payload":{"clock_id":"c","delta":1}}"#;
        assert!(parse_effect(nested).is_err());
        let external = r#"{"Clock":{"clock_id":"c","delta":1},"as":"npc:mira","intent_id":"i"}"#;
        assert!(parse_effect(external).is_err());
    }

    #[test]
    fn memory_and_migrate_lines_keep_their_key_order() {
        let memory = serde_json::to_string(&MemoryLine {
            round_index: 3,
            actor_id: "mi\"ra".to_string(),
            intent_id: "abc".to_string(),
            fact_ids: vec!["f".to_string()],
            tags: Vec::new(),
            text: None,
        })
        .unwrap();
        assert_eq!(
            memory,
            r#"{"round_index":3,"actor_id":"mi\"ra","intent_id":"abc","fact_ids":["f"],"tags":[],"text":null}"#
        );
        let migrate = crate::store::journal::emit_record(&crate::store::journal::migrate_record(
            "blake2s:aa",
            "blake2s:bb",
            0,
            1,
            "blake2s:cc",
            &[],
            false,
        ));
        assert_eq!(
            migrate,
            "{\"kind\":\"migrate\",\"pack_rev_before\":\"blake2s:aa\",\"pack_rev_after\":\"blake2s:bb\",\"state_rev_before\":0,\"state_rev_after\":1,\"state_hash\":\"blake2s:cc\"}\n"
        );
    }
}
