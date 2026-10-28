//! Read and write the save.

use serde::Deserialize;
use serde_saphyr::SerializerOptions;

use crate::diagnostic::{coded, locate_nth, parse_file, schema, tag_diagnostic, Diagnostic};
use crate::effect::Effect;
use crate::error::Reason;
use crate::model::{sort_unique, MemoryLine, Now};

#[allow(non_snake_case)]
pub(crate) fn SAVE_EMIT_OPTIONS() -> SerializerOptions {
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

pub fn load_save(text: &str) -> Result<Now, Vec<Diagnostic>> {
    const FILE: &str = "state/save.yaml";
    if let Some(diag) = tag_diagnostic(text, crate::diagnostic::NOW_SCHEMA, FILE, 0) {
        return Err(vec![diag]);
    }
    // Check the rules version before the full shape, so a save from older rules
    // (for example one that still carries `presence`) is refused as RULES_REV.
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
    let now: Now = parse_file(FILE, text, 0)?;
    let diags = occupancy_problems(&now);
    if diags.is_empty() {
        Ok(now)
    } else {
        Err(diags)
    }
}

pub fn emit_save(now: &Now) -> Result<String, Reason> {
    let mut canonical = now.clone();
    canonical.schema = crate::diagnostic::NOW_SCHEMA.to_string();
    canonical.rules_rev = super::RULES_REV;
    canonicalize(&mut canonical);
    let text = serde_saphyr::to_string_with_options(&canonical, SAVE_EMIT_OPTIONS())
        .map_err(|_| Reason::Schema)?;
    Ok(one_trailing_lf(text))
}

fn rules_rev_diagnostic(text: &str) -> Diagnostic {
    let mut diag = coded(
        "state/save.yaml",
        Reason::RulesRev,
        "this save was written by different rules",
        "start a fresh save",
    )
    .with_path("rules_rev");
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

pub fn canonicalize(now: &mut Now) {
    sort_unique(&mut now.presence.actors_present);
    now.messages.sort_by(|a, b| a.id.cmp(&b.id));
    for message in &mut now.messages {
        sort_unique(&mut message.to);
        sort_unique(&mut message.fact_ids);
    }
    let occupancy = rebuilt_occupancy(now);
    for (place_id, place) in &mut now.places {
        sort_unique(&mut place.notes);
        place.actors = occupancy.get(place_id).cloned().unwrap_or_default();
    }
    for actor in now.actors.values_mut() {
        sort_unique(actor.flags_mut());
        sort_unique(actor.known_facts_mut());
    }
    sort_unique(&mut now.flags);
    for clock in now.clocks.values_mut() {
        sort_unique(&mut clock.owners);
    }
}

fn rebuilt_occupancy(now: &Now) -> std::collections::BTreeMap<String, Vec<String>> {
    let mut out: std::collections::BTreeMap<String, Vec<String>> = now
        .places
        .keys()
        .map(|id| (id.clone(), Vec::new()))
        .collect();
    for (actor_id, actor) in &now.actors {
        if let Some(list) = out.get_mut(actor.place()) {
            list.push(actor_id.clone());
        }
    }
    for list in out.values_mut() {
        list.sort();
    }
    out
}

fn occupancy_problems(now: &Now) -> Vec<Diagnostic> {
    const FILE: &str = "state/save.yaml";
    let mut diags = Vec::new();
    for (id, actor) in &now.actors {
        if now.places.contains_key(actor.place()) {
            continue;
        }
        diags.push(
            schema(
                FILE,
                &format!(
                    "actor '{id}' is in place '{}', which is not in places",
                    actor.place()
                ),
                "add the place or move the actor",
            )
            .with_path(format!("actors.{id}.place")),
        );
    }
    let expected = rebuilt_occupancy(now);
    for (place_id, place) in &now.places {
        let mut listed = place.actors.clone();
        listed.sort();
        let got = expected.get(place_id).map(Vec::as_slice).unwrap_or(&[]);
        if listed != got {
            diags.push(
                schema(
                    FILE,
                    &format!(
                        "place '{place_id}' lists actors that do not match where the actors stand"
                    ),
                    "list the actors whose place is this one, or run spook migrate",
                )
                .with_path(format!("places.{place_id}.actors")),
            );
        }
    }
    diags
}

/// Flat effect object. Key order is `class`, `as`, `intent_id`, then payload fields.
pub fn emit_effect(effect: &Effect) -> String {
    serde_json::to_string(effect).expect("effect json")
}

pub(crate) fn emit_memory_line(line: &MemoryLine) -> String {
    serde_json::to_string(line).expect("memory line")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn parse_effect(text: &str) -> Result<Effect, Reason> {
        let record: crate::effect::EffectRecord =
            serde_json::from_str(text).map_err(|_| Reason::Schema)?;
        Effect::try_from(record)
    }

    #[test]
    fn docks_now_round_trips_through_emitter() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../packs/oneshot-docks/state/save.yaml");
        let original = fs::read_to_string(&path).unwrap();
        let now = load_save(&original).unwrap();
        let emitted = emit_save(&now).unwrap();
        assert_eq!(emitted, original);
        assert_eq!(load_save(&emitted).unwrap(), now);
    }

    fn now_text(tag: &str) -> String {
        format!(
            "\
schema: {tag}
shard_id: ex
pack_rev: blake2s:00
rules_rev: {rules}
state_rev: 0
round_index: 0
world_time: 0
last_tick_at: \"2026-01-01T00:00:00Z\"
pause:
  named_plot_clocks: paused
places:
  gate:
    items: {{}}
    notes: []
    actors: []
actors: {{}}
clocks: {{}}
messages: []
",
            rules = crate::store::RULES_REV,
        )
    }

    #[test]
    fn an_unknown_tag_and_the_old_versioned_tag_are_schema() {
        assert!(load_save(&now_text("now")).is_ok());
        let old = load_save(&now_text(concat!("now/", "v0"))).unwrap_err();
        assert!(old[0].message.contains("no version"), "{old:?}");
        assert_eq!(crate::diagnostic::reason_of(old), Reason::Schema);
        let bogus = load_save(&now_text("bogus")).unwrap_err();
        assert!(bogus[0].message.contains("does not use"), "{bogus:?}");
        assert_eq!(crate::diagnostic::reason_of(bogus), Reason::Schema);
    }

    #[test]
    fn emit_save_writes_the_now_tag() {
        let mut now = load_save(&now_text("now")).unwrap();
        now.schema = concat!("now/", "v0").to_string();
        let emitted = emit_save(&now).unwrap();
        assert!(emitted.starts_with("schema: now\n"), "{emitted}");
        assert_eq!(load_save(&emitted).unwrap().schema, "now");
    }

    use crate::effect::EffectKind;
    use crate::error::Reason;
    use crate::intent::ViewerId;
    use crate::model::MemoryLine;

    fn effect(class: EffectKind, viewer: ViewerId, intent_id: &str) -> super::Effect {
        super::Effect {
            class,
            as_viewer: viewer,
            intent_id: intent_id.to_string(),
        }
    }

    fn npc(id: &str) -> ViewerId {
        ViewerId::Npc(id.to_string())
    }

    fn player(id: &str) -> ViewerId {
        ViewerId::Player(id.to_string())
    }

    #[test]
    fn effect_json_key_order_is_class_as_intent_then_payload_fields() {
        let cases = [
            (
                effect(
                    EffectKind::Clock {
                        clock_id: "shop_day".to_string(),
                        delta: 1,
                        start: false,
                    },
                    npc("mira"),
                    "i1",
                ),
                r#"{"class":"clock","as":"npc:mira","intent_id":"i1","clock_id":"shop_day","delta":1}"#,
            ),
            (
                effect(
                    EffectKind::Meter {
                        actor_id: "mira".to_string(),
                        key: "rest".to_string(),
                        write: crate::meter::MeterWrite::Base(1),
                    },
                    npc("mira"),
                    "i2",
                ),
                r#"{"class":"meter","as":"npc:mira","intent_id":"i2","actor_id":"mira","key":"rest","write":"base","delta":1}"#,
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
                r#"{"class":"item","as":"npc:mira","intent_id":"i3","place":"docks_shop","id":"lamp_oil","delta":4,"max":8}"#,
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
                r#"{"class":"location","as":"npc:bram","intent_id":"i4","actor_id":"bram","from":"docks_street","to":"docks_shop"}"#,
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
                r#"{"class":"memory","as":"npc:mira","intent_id":"i7","actor_id":"mira","fact_ids":["fact_a","fact_b"],"tags":["heard"],"text":"oil"}"#,
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
                r#"{"class":"memory","as":"npc:mira","intent_id":"i7b","actor_id":"mira","fact_ids":[],"tags":[]}"#,
            ),
            (
                effect(
                    EffectKind::Meter {
                        actor_id: "pc_rowan".to_string(),
                        key: "grit".to_string(),
                        write: crate::meter::MeterWrite::Snap,
                    },
                    player("pc_rowan"),
                    "i9",
                ),
                r#"{"class":"meter","as":"player:pc_rowan","intent_id":"i9","actor_id":"pc_rowan","key":"grit","write":"snap"}"#,
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
                r#"{"class":"flag","as":"npc:mira","intent_id":"i10","actor_id":"mira","key":"marked","value":true}"#,
            ),
            (
                effect(
                    EffectKind::Message {
                        message_id: "note_mira_door".to_string(),
                        to: vec!["pc_rowan".to_string()],
                        place: Some("docks_shop".to_string()),
                        fact_ids: Vec::new(),
                        text: "say \"hi\"".to_string(),
                    },
                    npc("mira"),
                    "i12",
                ),
                r#"{"class":"message","as":"npc:mira","intent_id":"i12","message_id":"note_mira_door","to":["pc_rowan"],"place":"docks_shop","fact_ids":[],"text":"say \"hi\""}"#,
            ),
            (
                effect(
                    EffectKind::Message {
                        message_id: "n".to_string(),
                        to: Vec::new(),
                        place: None,
                        fact_ids: vec!["f".to_string()],
                        text: "x".to_string(),
                    },
                    npc("mira"),
                    "i12b",
                ),
                r#"{"class":"message","as":"npc:mira","intent_id":"i12b","message_id":"n","to":[],"fact_ids":["f"],"text":"x"}"#,
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
                r#"{"class":"reveal","as":"player:pc_nell","intent_id":"i13","actor_id":"pc_nell","fact":"fact_trapdoor"}"#,
            ),
        ];
        for (effect, expected) in cases {
            let encoded = super::emit_effect(&effect);
            assert_eq!(encoded, expected);
            assert!(!encoded.contains("\"payload\""), "{encoded}");
            assert_eq!(parse_effect(&encoded).unwrap(), effect);
        }
        for name in ["need", "resource", "inventory"] {
            let value = serde_json::json!({
                "class": name,
                "as": "npc:mira",
                "intent_id": "i",
            });
            assert_eq!(
                parse_effect(&value.to_string()).unwrap_err(),
                Reason::IllegalClass
            );
        }
        let snap_delta = serde_json::json!({
            "class": "meter",
            "as": "npc:mira",
            "intent_id": "i",
            "actor_id": "mira",
            "key": "rest",
            "write": "snap",
            "delta": 1,
        });
        assert_eq!(
            parse_effect(&snap_delta.to_string()).unwrap_err(),
            Reason::Schema
        );
        let missing_delta = serde_json::json!({
            "class": "meter",
            "as": "npc:mira",
            "intent_id": "i",
            "actor_id": "mira",
            "key": "rest",
            "write": "base",
        });
        assert_eq!(
            parse_effect(&missing_delta.to_string()).unwrap_err(),
            Reason::Schema
        );
    }

    #[test]
    fn effect_payload_object_keeps_declaration_order_and_a_negative_delta() {
        let effect = parse_effect(
            r#"{"class":"item","as":"npc:n","intent_id":"i","place":"p","id":"s","delta":-1}"#,
        )
        .unwrap();
        assert_eq!(
            crate::effect::payload_json(&effect),
            r#"{"place":"p","id":"s","delta":-1}"#
        );
    }

    #[test]
    fn nested_payload_and_external_tag_are_not_effects() {
        let nested = r#"{"class":"clock","as":"npc:mira","intent_id":"i","payload":{"clock_id":"c","delta":1}}"#;
        assert!(parse_effect(nested).is_err());
        let external = r#"{"Clock":{"clock_id":"c","delta":1},"as":"npc:mira","intent_id":"i"}"#;
        assert!(parse_effect(external).is_err());
    }

    #[test]
    fn memory_and_migrate_lines_keep_their_key_order() {
        let memory = super::emit_memory_line(&MemoryLine {
            round_index: 3,
            actor_id: "mi\"ra".to_string(),
            intent_id: "abc".to_string(),
            fact_ids: vec!["f".to_string()],
            tags: Vec::new(),
            text: None,
        });
        assert_eq!(
            memory,
            r#"{"round_index":3,"actor_id":"mi\"ra","intent_id":"abc","fact_ids":["f"],"tags":[],"text":null}"#
        );
        let migrate = crate::journal::emit_entry(&crate::journal::migrate_entry(
            "blake2s:aa",
            "blake2s:bb",
            0,
            1,
            "blake2s:cc",
            &[],
        ));
        assert_eq!(
            migrate,
            "{\"kind\":\"migrate\",\"pack_rev_before\":\"blake2s:aa\",\"pack_rev_after\":\"blake2s:bb\",\"state_rev_before\":0,\"state_rev_after\":1,\"now_hash\":\"blake2s:cc\"}\n"
        );
    }
}
