use serde::Serialize;
use serde_json::{Map, Value};
use serde_saphyr::SerializerOptions;

use crate::effect::{Effect, EffectClass, EffectPayload};
use crate::error::Reason;
use crate::model::{sort_unique, MemoryLine, Now, ProposalDoc, ViewerId};

#[allow(non_snake_case)]
pub fn NOW_EMIT_OPTIONS() -> SerializerOptions {
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

pub fn load_now(text: &str) -> Result<Now, Reason> {
    let now: Now = serde_saphyr::from_str(text).map_err(|_| Reason::Schema)?;
    if now.schema != "now/v0" {
        return Err(Reason::Schema);
    }
    check_occupancy(&now)?;
    Ok(now)
}

pub fn emit_now(now: &Now) -> Result<String, Reason> {
    let mut canonical = now.clone();
    canonicalize(&mut canonical);
    let text = serde_saphyr::to_string_with_options(&canonical, NOW_EMIT_OPTIONS())
        .map_err(|_| Reason::Schema)?;
    Ok(one_trailing_lf(text))
}

pub fn emit_proposal(doc: &ProposalDoc) -> Result<String, Reason> {
    if doc.schema != "proposal/v0" {
        return Err(Reason::Schema);
    }
    let text = serde_saphyr::to_string_with_options(doc, NOW_EMIT_OPTIONS())
        .map_err(|_| Reason::Schema)?;
    Ok(one_trailing_lf(text))
}

pub fn emit_rumors(now: &Now) -> Result<String, Reason> {
    let mut rumors = now.rumors.clone();
    rumors.sort_by(|a, b| a.id.cmp(&b.id));
    for rumor in &mut rumors {
        let mut facts = rumor.fact_ids.clone();
        sort_unique(&mut facts);
        rumor.fact_ids = facts;
        let mut known = rumor.known_by.clone();
        sort_unique(&mut known);
        rumor.known_by = known;
    }
    let text = serde_saphyr::to_string_with_options(&rumors, NOW_EMIT_OPTIONS())
        .map_err(|_| Reason::Schema)?;
    Ok(one_trailing_lf(text))
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
    sort_unique(&mut now.proposals);
    now.rumors.sort_by(|a, b| a.id.cmp(&b.id));
    for rumor in &mut now.rumors {
        sort_unique(&mut rumor.fact_ids);
        sort_unique(&mut rumor.known_by);
    }
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

fn check_occupancy(now: &Now) -> Result<(), Reason> {
    for actor in now.actors.values() {
        if !now.places.contains_key(actor.place()) {
            return Err(Reason::Schema);
        }
    }
    let expected = rebuilt_occupancy(now);
    for (place_id, place) in &now.places {
        let mut listed = place.actors.clone();
        listed.sort();
        let got = expected.get(place_id).map(Vec::as_slice).unwrap_or(&[]);
        if listed != got {
            return Err(Reason::Schema);
        }
    }
    Ok(())
}

/// Flat effect object. Key order is `class`, `as`, `intent_id`, then payload fields.
pub fn emit_effect(effect: &Effect) -> String {
    serde_json::to_string(effect).expect("effect json")
}

/// Payload fields only, in `EffectClass` declaration order. Absent options are omitted.
pub fn emit_effect_payload(effect: &Effect) -> String {
    serde_json::to_string(&EffectPayload(&effect.class)).expect("effect payload")
}

pub fn parse_tick_effects(text: &str) -> Result<(u64, Vec<Effect>), Reason> {
    let value: Value = serde_json::from_str(text.trim()).map_err(|_| Reason::Schema)?;
    let object = value.as_object().ok_or(Reason::Schema)?;
    let tick_index = object
        .get("tick_index")
        .and_then(Value::as_u64)
        .ok_or(Reason::Schema)?;
    let effects = object
        .get("effects")
        .and_then(Value::as_array)
        .ok_or(Reason::Schema)?;
    let mut parsed = Vec::with_capacity(effects.len());
    for effect in effects {
        parsed.push(parse_effect(effect)?);
    }
    Ok((tick_index, parsed))
}

pub fn parse_effect(value: &Value) -> Result<Effect, Reason> {
    let object = value.as_object().ok_or(Reason::Schema)?;
    let name = object
        .get("class")
        .and_then(Value::as_str)
        .ok_or(Reason::Schema)?;
    let token = object
        .get("as")
        .and_then(Value::as_str)
        .ok_or(Reason::Schema)?;
    let as_viewer = ViewerId::parse(token).map_err(|_| Reason::Schema)?;
    let intent_id = req_str(object, "intent_id")?;
    let class = parse_class(name, object)?;
    Ok(Effect {
        class,
        as_viewer,
        intent_id,
    })
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

pub fn emit_migrate_line(
    pack_rev_before: &str,
    pack_rev_after: &str,
    state_rev_before: u64,
    state_rev_after: u64,
) -> String {
    let json = MigrateLineJson {
        op: "migrate",
        pack_rev_before,
        pack_rev_after,
        state_rev_before,
        state_rev_after,
    };
    serde_json::to_string(&json).expect("migrate line")
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

#[derive(Serialize)]
struct MigrateLineJson<'a> {
    op: &'a str,
    pack_rev_before: &'a str,
    pack_rev_after: &'a str,
    state_rev_before: u64,
    state_rev_after: u64,
}

fn parse_class(name: &str, object: &Map<String, Value>) -> Result<EffectClass, Reason> {
    match name {
        "clock" => {
            allow(object, &["clock_id", "delta"])?;
            Ok(EffectClass::Clock {
                clock_id: req_str(object, "clock_id")?,
                delta: req_i64(object, "delta")?,
            })
        }
        "need" => {
            allow(object, &["actor_id", "key", "delta"])?;
            Ok(EffectClass::Need {
                actor_id: req_str(object, "actor_id")?,
                key: req_str(object, "key")?,
                delta: req_i64(object, "delta")?,
            })
        }
        "stock" => {
            allow(object, &["place", "sku", "delta"])?;
            Ok(EffectClass::Stock {
                place: req_str(object, "place")?,
                sku: req_str(object, "sku")?,
                delta: req_i64(object, "delta")?,
            })
        }
        "location" => {
            allow(object, &["actor_id", "from", "to"])?;
            Ok(EffectClass::Location {
                actor_id: req_str(object, "actor_id")?,
                from: req_str(object, "from")?,
                to: req_str(object, "to")?,
            })
        }
        "relationship" => {
            allow(object, &["a", "b", "delta"])?;
            Ok(EffectClass::Relationship {
                a: req_str(object, "a")?,
                b: req_str(object, "b")?,
                delta: req_i64(object, "delta")?,
            })
        }
        "reputation" => {
            allow(object, &["actor_id", "faction", "delta"])?;
            Ok(EffectClass::Reputation {
                actor_id: req_str(object, "actor_id")?,
                faction: req_str(object, "faction")?,
                delta: req_i64(object, "delta")?,
            })
        }
        "memory" => {
            allow(object, &["actor_id", "fact_ids", "tags", "text"])?;
            Ok(EffectClass::Memory {
                actor_id: req_str(object, "actor_id")?,
                fact_ids: req_strs(object, "fact_ids")?,
                tags: req_strs(object, "tags")?,
                text: opt_str(object, "text")?,
            })
        }
        "rumor" => {
            allow(object, &["rumor_id", "fact_ids", "from", "to", "heat"])?;
            Ok(EffectClass::Rumor {
                rumor_id: req_str(object, "rumor_id")?,
                fact_ids: req_strs(object, "fact_ids")?,
                from: req_str(object, "from")?,
                to: req_str(object, "to")?,
                heat: req_i64(object, "heat")?,
            })
        }
        "resource" => {
            allow(object, &["actor_id", "key", "delta"])?;
            Ok(EffectClass::Resource {
                actor_id: req_str(object, "actor_id")?,
                key: req_str(object, "key")?,
                delta: req_i64(object, "delta")?,
            })
        }
        "flag" => {
            allow(object, &["actor_id", "key", "value"])?;
            Ok(EffectClass::Flag {
                actor_id: req_str(object, "actor_id")?,
                key: req_str(object, "key")?,
                value: req_bool(object, "value")?,
            })
        }
        "proposal" => {
            allow(object, &["proposal_id", "kind", "target", "summary"])?;
            Ok(EffectClass::Proposal {
                proposal_id: req_str(object, "proposal_id")?,
                kind: req_str(object, "kind")?,
                target: req_str(object, "target")?,
                summary: req_str(object, "summary")?,
            })
        }
        "message" => {
            allow(object, &["message_id", "to", "place", "fact_ids", "text"])?;
            Ok(EffectClass::Message {
                message_id: req_str(object, "message_id")?,
                to: req_strs(object, "to")?,
                place: opt_str(object, "place")?,
                fact_ids: req_strs(object, "fact_ids")?,
                text: req_str(object, "text")?,
            })
        }
        _ => Err(Reason::Schema),
    }
}

fn allow(object: &Map<String, Value>, fields: &[&str]) -> Result<(), Reason> {
    for key in object.keys() {
        if key == "class" || key == "as" || key == "intent_id" || fields.contains(&key.as_str()) {
            continue;
        }
        return Err(Reason::Schema);
    }
    Ok(())
}

fn req_str(object: &Map<String, Value>, key: &str) -> Result<String, Reason> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or(Reason::Schema)
}

fn opt_str(object: &Map<String, Value>, key: &str) -> Result<Option<String>, Reason> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(Reason::Schema),
    }
}

fn req_i64(object: &Map<String, Value>, key: &str) -> Result<i64, Reason> {
    object
        .get(key)
        .and_then(Value::as_i64)
        .ok_or(Reason::Schema)
}

fn req_bool(object: &Map<String, Value>, key: &str) -> Result<bool, Reason> {
    object
        .get(key)
        .and_then(Value::as_bool)
        .ok_or(Reason::Schema)
}

fn req_strs(object: &Map<String, Value>, key: &str) -> Result<Vec<String>, Reason> {
    let array = object
        .get(key)
        .and_then(Value::as_array)
        .ok_or(Reason::Schema)?;
    let mut out = Vec::with_capacity(array.len());
    for item in array {
        out.push(item.as_str().ok_or(Reason::Schema)?.to_string());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn docks_now_round_trips_through_emitter() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../packs/oneshot-docks/state/now.yaml");
        let original = fs::read_to_string(&path).unwrap();
        let now = load_now(&original).unwrap();
        let emitted = emit_now(&now).unwrap();
        assert_eq!(emitted, original);
    }

    use crate::effect::EffectClass;
    use crate::model::{MemoryLine, ViewerId};

    fn effect(class: EffectClass, viewer: ViewerId, intent_id: &str) -> super::Effect {
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
                    EffectClass::Clock {
                        clock_id: "shop_day".to_string(),
                        delta: 1,
                    },
                    npc("mira"),
                    "i1",
                ),
                r#"{"class":"clock","as":"npc:mira","intent_id":"i1","clock_id":"shop_day","delta":1}"#,
            ),
            (
                effect(
                    EffectClass::Need {
                        actor_id: "pc_rowan".to_string(),
                        key: "rest".to_string(),
                        delta: -2,
                    },
                    player("pc_rowan"),
                    "i2",
                ),
                r#"{"class":"need","as":"player:pc_rowan","intent_id":"i2","actor_id":"pc_rowan","key":"rest","delta":-2}"#,
            ),
            (
                effect(
                    EffectClass::Stock {
                        place: "docks_shop".to_string(),
                        sku: "lamp_oil".to_string(),
                        delta: 4,
                    },
                    npc("mira"),
                    "i3",
                ),
                r#"{"class":"stock","as":"npc:mira","intent_id":"i3","place":"docks_shop","sku":"lamp_oil","delta":4}"#,
            ),
            (
                effect(
                    EffectClass::Location {
                        actor_id: "bram".to_string(),
                        from: "docks_street".to_string(),
                        to: "docks_shop".to_string(),
                    },
                    npc("bram"),
                    "i4",
                ),
                r#"{"class":"location","as":"npc:bram","intent_id":"i4","actor_id":"bram","from":"docks_street","to":"docks_shop"}"#,
            ),
            (
                effect(
                    EffectClass::Relationship {
                        a: "mira".to_string(),
                        b: "bram".to_string(),
                        delta: 1,
                    },
                    npc("mira"),
                    "i5",
                ),
                r#"{"class":"relationship","as":"npc:mira","intent_id":"i5","a":"mira","b":"bram","delta":1}"#,
            ),
            (
                effect(
                    EffectClass::Reputation {
                        actor_id: "mira".to_string(),
                        faction: "dockers".to_string(),
                        delta: -1,
                    },
                    npc("mira"),
                    "i6",
                ),
                r#"{"class":"reputation","as":"npc:mira","intent_id":"i6","actor_id":"mira","faction":"dockers","delta":-1}"#,
            ),
            (
                effect(
                    EffectClass::Memory {
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
                    EffectClass::Memory {
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
                    EffectClass::Rumor {
                        rumor_id: "r_184".to_string(),
                        fact_ids: vec!["fact_oil_on_shelf".to_string()],
                        from: "bram".to_string(),
                        to: "aldric".to_string(),
                        heat: 1,
                    },
                    npc("bram"),
                    "i8",
                ),
                r#"{"class":"rumor","as":"npc:bram","intent_id":"i8","rumor_id":"r_184","fact_ids":["fact_oil_on_shelf"],"from":"bram","to":"aldric","heat":1}"#,
            ),
            (
                effect(
                    EffectClass::Resource {
                        actor_id: "mira".to_string(),
                        key: "coin".to_string(),
                        delta: 3,
                    },
                    npc("mira"),
                    "i9",
                ),
                r#"{"class":"resource","as":"npc:mira","intent_id":"i9","actor_id":"mira","key":"coin","delta":3}"#,
            ),
            (
                effect(
                    EffectClass::Flag {
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
                    EffectClass::Proposal {
                        proposal_id: "season_advance".to_string(),
                        kind: "other".to_string(),
                        target: String::new(),
                        summary: "season_advance".to_string(),
                    },
                    player("pc_rowan"),
                    "i11",
                ),
                r#"{"class":"proposal","as":"player:pc_rowan","intent_id":"i11","proposal_id":"season_advance","kind":"other","target":"","summary":"season_advance"}"#,
            ),
            (
                effect(
                    EffectClass::Message {
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
                    EffectClass::Message {
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
        ];
        for (effect, expected) in cases {
            let encoded = super::emit_effect(&effect);
            assert_eq!(encoded, expected);
            assert!(!encoded.contains("\"payload\""), "{encoded}");
            assert_eq!(
                super::parse_effect(&serde_json::from_str(&encoded).unwrap()).unwrap(),
                effect
            );
        }
    }

    #[test]
    fn effect_payload_object_keeps_declaration_order_and_a_negative_delta() {
        let text = r#"{"tick_index":1,"effects":[{"class":"stock","as":"npc:n","intent_id":"i","place":"p","sku":"s","delta":-1}]}"#;
        let (tick_index, effects) = super::parse_tick_effects(text).unwrap();
        assert_eq!(tick_index, 1);
        assert_eq!(
            super::emit_effect_payload(&effects[0]),
            r#"{"place":"p","sku":"s","delta":-1}"#
        );
    }

    #[test]
    fn nested_payload_and_external_tag_are_not_effects() {
        let nested = r#"{"class":"clock","as":"npc:mira","intent_id":"i","payload":{"clock_id":"c","delta":1}}"#;
        assert!(super::parse_effect(&serde_json::from_str(nested).unwrap()).is_err());
        let external = r#"{"Clock":{"clock_id":"c","delta":1},"as":"npc:mira","intent_id":"i"}"#;
        assert!(super::parse_effect(&serde_json::from_str(external).unwrap()).is_err());
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
        let migrate = super::emit_migrate_line("blake2s:aa", "blake2s:bb", 0, 1);
        assert_eq!(
            migrate,
            r#"{"op":"migrate","pack_rev_before":"blake2s:aa","pack_rev_after":"blake2s:bb","state_rev_before":0,"state_rev_after":1}"#
        );
    }
}
