use serde::{Deserialize, Serialize};
use serde_saphyr::SerializerOptions;

use crate::diagnostic::{coded, locate_nth, parse_file, schema, tag_diagnostic, Diagnostic};
use crate::effect::Effect;
use crate::error::Reason;
use crate::model::{sort_unique, MemoryLine, Now};

#[allow(non_snake_case)]
pub fn SAVE_EMIT_OPTIONS() -> SerializerOptions {
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
    if let Some(diag) = tag_diagnostic(text, crate::format::NOW, FILE, 0) {
        return Err(vec![diag]);
    }
    let now: Now = parse_file(FILE, text, 0)?;
    if super::require_rules_rev(now.rules_rev).is_err() {
        return Err(vec![rules_rev_diagnostic(text)]);
    }
    let mut diags = Vec::new();
    if now.schema != crate::format::NOW {
        diags.push(
            schema(
                FILE,
                &format!("schema is '{}', which this file does not use", now.schema),
                "write 'schema: now' with no version suffix",
            )
            .with_path("schema"),
        );
    }
    diags.extend(occupancy_problems(&now));
    if diags.is_empty() {
        Ok(now)
    } else {
        Err(diags)
    }
}

pub fn emit_save(now: &Now) -> Result<String, Reason> {
    let mut canonical = now.clone();
    canonical.schema = crate::format::NOW.to_string();
    canonical.rules_rev = super::RULES_REV;
    canonicalize(&mut canonical);
    let text = serde_saphyr::to_string_with_options(&canonical, SAVE_EMIT_OPTIONS())
        .map_err(|_| Reason::Schema)?;
    Ok(one_trailing_lf(text))
}

/// Drop a top-level `presence` block. Old saves still carry one.
pub fn without_presence(yaml: &str) -> String {
    let mut out = String::new();
    let mut skipping = false;
    for line in yaml.split_inclusive('\n') {
        let text = line.trim_end_matches(['\n', '\r']);
        if !skipping && text == "presence:" {
            skipping = true;
            continue;
        }
        if skipping {
            if text.is_empty() || text.starts_with(' ') || text.starts_with('\t') {
                continue;
            }
            skipping = false;
        }
        out.push_str(line);
    }
    out
}

/// Put `presence` back where the old emitter wrote it, before `pause`.
pub fn with_presence(yaml: &str, humans: u32, actors: &[String]) -> String {
    let stripped = without_presence(yaml);
    let block = presence_block(humans, actors);
    let mut out = String::new();
    let mut inserted = false;
    for line in stripped.split_inclusive('\n') {
        let text = line.trim_end_matches(['\n', '\r']);
        if !inserted && text == "pause:" {
            out.push_str(&block);
            inserted = true;
        }
        out.push_str(line);
    }
    if !inserted {
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&block);
    }
    out
}

fn presence_block(humans: u32, actors: &[String]) -> String {
    let mut actors = actors.to_vec();
    actors.sort();
    actors.dedup();
    let mut block = format!("presence:\n  humans: {humans}\n  actors_present:");
    if actors.is_empty() {
        block.push_str(" []\n");
    } else {
        block.push('\n');
        for id in actors {
            block.push_str("    - ");
            block.push_str(&id);
            block.push('\n');
        }
    }
    block
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

/// Refuse a tick record whose `rules_rev` is missing or not the engine's.
pub fn require_recorded_rules(text: &str) -> Result<(), Reason> {
    #[derive(Deserialize)]
    struct RulesProbe {
        #[serde(default)]
        rules_rev: Option<u64>,
    }
    let probe: RulesProbe = serde_json::from_str(text.trim_end()).map_err(|_| Reason::Schema)?;
    match probe.rules_rev {
        Some(rev) => super::require_rules_rev(rev),
        None => Err(Reason::RulesRev),
    }
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

/// Payload fields only, in declaration order. Absent options are omitted.
pub fn emit_effect_payload(effect: &Effect) -> String {
    super::records::payload_json(effect)
}

pub fn parse_tick_effects(text: &str) -> Result<(u64, Vec<Effect>), Reason> {
    #[derive(Deserialize)]
    struct EffectsOnly {
        tick_index: u64,
        effects: Vec<Effect>,
    }
    let parsed: EffectsOnly = serde_json::from_str(text.trim())
        .map_err(|err| super::records::effect_reason(&err.to_string()))?;
    Ok((parsed.tick_index, parsed.effects))
}

pub fn parse_effect(text: &str) -> Result<Effect, Reason> {
    serde_json::from_str(text).map_err(|err| super::records::effect_reason(&err.to_string()))
}

pub fn emit_memory_line(line: &MemoryLine) -> String {
    let json = MemoryLineJson {
        tick_index: line.tick_index,
        actor_id: &line.actor_id,
        intent_id: &line.intent_id,
        fact_ids: &line.fact_ids,
        tags: &line.tags,
        text: line.text.as_deref(),
    };
    serde_json::to_string(&json).expect("memory line")
}

#[derive(Serialize)]
struct MemoryLineJson<'a> {
    tick_index: u64,
    actor_id: &'a str,
    intent_id: &'a str,
    fact_ids: &'a [String],
    tags: &'a [String],
    text: Option<&'a str>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn a_tick_with_the_wrong_rules_rev_is_refused_while_the_save_matches() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../packs/oneshot-docks/state/save.yaml");
        let save = fs::read_to_string(&path).unwrap();
        assert!(load_save(&save).is_ok(), "{save}");
        let wrong = crate::store::RULES_REV + 1;
        let record = format!(r#"{{"tick_index":1,"rules_rev":{wrong}}}"#);
        assert_eq!(require_recorded_rules(&record), Err(Reason::RulesRev));
        let current = format!(
            r#"{{"tick_index":1,"rules_rev":{}}}"#,
            crate::store::RULES_REV
        );
        assert_eq!(require_recorded_rules(&current), Ok(()));
    }

    #[test]
    fn docks_now_round_trips_through_emitter() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../packs/oneshot-docks/state/save.yaml");
        let original = fs::read_to_string(&path).unwrap();
        let now = load_save(&original).unwrap();
        let emitted = emit_save(&now).unwrap();
        assert_eq!(emitted, without_presence(&original));
        assert_eq!(
            with_presence(&emitted, now.presence.humans, &now.presence.actors_present),
            original
        );
        assert_eq!(load_save(&emitted).unwrap(), now);
    }

    /// Written by the emitter from before presence left the save, with two PCs seated.
    /// Old journal hashes cover these exact bytes, so `with_presence` must rebuild them.
    #[test]
    fn with_presence_rebuilds_an_old_seated_save() {
        let original = include_str!("../fixtures/old-save-seated.yaml");
        let now = load_save(original).unwrap();
        let emitted = emit_save(&now).unwrap();
        assert!(!emitted.contains("\npresence:"), "{emitted}");
        let actors = ["pc_wren".to_string(), "pc_rowan".to_string()];
        assert_eq!(with_presence(&emitted, 2, &actors), original);
    }

    fn now_text(tag: &str) -> String {
        format!(
            "\
schema: {tag}
shard_id: ex
pack_rev: blake2s:00
rules_rev: {rules}
state_rev: 0
tick_index: 0
world_time: 0
last_tick_at: \"2026-01-01T00:00:00Z\"
presence:
  humans: 0
  actors_present: []
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
            assert_eq!(super::parse_effect(&encoded).unwrap(), effect);
        }
        for name in ["need", "resource", "inventory"] {
            let value = serde_json::json!({
                "class": name,
                "as": "npc:mira",
                "intent_id": "i",
            });
            assert_eq!(
                super::parse_effect(&value.to_string()).unwrap_err(),
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
            super::parse_effect(&snap_delta.to_string()).unwrap_err(),
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
            super::parse_effect(&missing_delta.to_string()).unwrap_err(),
            Reason::Schema
        );
    }

    #[test]
    fn effect_payload_object_keeps_declaration_order_and_a_negative_delta() {
        let text = r#"{"tick_index":1,"effects":[{"class":"item","as":"npc:n","intent_id":"i","place":"p","id":"s","delta":-1}]}"#;
        let (tick_index, effects) = super::parse_tick_effects(text).unwrap();
        assert_eq!(tick_index, 1);
        assert_eq!(
            super::emit_effect_payload(&effects[0]),
            r#"{"place":"p","id":"s","delta":-1}"#
        );
    }

    #[test]
    fn nested_payload_and_external_tag_are_not_effects() {
        let nested = r#"{"class":"clock","as":"npc:mira","intent_id":"i","payload":{"clock_id":"c","delta":1}}"#;
        assert!(super::parse_effect(nested).is_err());
        let external = r#"{"Clock":{"clock_id":"c","delta":1},"as":"npc:mira","intent_id":"i"}"#;
        assert!(super::parse_effect(external).is_err());
    }

    #[test]
    fn memory_and_migrate_lines_keep_their_key_order() {
        let memory = super::emit_memory_line(&MemoryLine {
            tick_index: 3,
            actor_id: "mi\"ra".to_string(),
            intent_id: "abc".to_string(),
            fact_ids: vec!["f".to_string()],
            tags: Vec::new(),
            text: None,
        });
        assert_eq!(
            memory,
            r#"{"tick_index":3,"actor_id":"mi\"ra","intent_id":"abc","fact_ids":["f"],"tags":[],"text":null}"#
        );
        let migrate = crate::journal::emit_migrate("blake2s:aa", "blake2s:bb", 0, 1, "blake2s:cc");
        assert_eq!(
            migrate,
            "{\"kind\":\"migrate\",\"pack_rev_before\":\"blake2s:aa\",\"pack_rev_after\":\"blake2s:bb\",\"state_rev_before\":0,\"state_rev_after\":1,\"now_hash\":\"blake2s:cc\"}\n"
        );
    }
}
