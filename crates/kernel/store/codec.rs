use serde::Serialize;
use serde_json::{Map, Value};
use serde_saphyr::SerializerOptions;

use crate::diagnostic::{coded, locate_nth, parse_file, schema, tag_diagnostic, Diagnostic};
use crate::effect::{Effect, EffectClass, EffectPayload};
use crate::error::Reason;
use crate::intent::{ActorKind, Intent, IntentArgs, Source, Target, ViewerId};
use crate::meter::MeterWrite;
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
    let value: Value = serde_json::from_str(text.trim_end()).map_err(|_| Reason::Schema)?;
    let rev = value
        .as_object()
        .and_then(|object| object.get("rules_rev"))
        .and_then(Value::as_u64);
    match rev {
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

/// Header and intents of one tick JSONL object. Trailing whitespace is ignored.
/// Other fields, including `effects`, stay on `parse_tick_effects`.
pub struct RecordedLine {
    pub tick_index: u64,
    pub state_rev_after: u64,
    pub presence_humans: u32,
    pub seed: String,
    pub last_tick_at: String,
    pub intents: Vec<Intent>,
}

pub fn parse_recorded_line(text: &str) -> Result<RecordedLine, Reason> {
    let value: Value = serde_json::from_str(text.trim_end()).map_err(|_| Reason::Schema)?;
    let object = value.as_object().ok_or(Reason::Schema)?;
    let intents = object
        .get("intents")
        .and_then(Value::as_array)
        .ok_or(Reason::Schema)?;
    let mut parsed = Vec::new();
    for intent in intents {
        parsed.push(parse_intent(intent)?);
    }
    Ok(RecordedLine {
        tick_index: req_u64(object, "tick_index")?,
        state_rev_after: req_u64(object, "state_rev_after")?,
        presence_humans: req_u64(object, "presence_humans")?
            .try_into()
            .map_err(|_| Reason::Schema)?,
        seed: req_str(object, "seed")?,
        last_tick_at: req_str(object, "last_tick_at")?,
        intents: parsed,
    })
}

fn parse_intent(value: &Value) -> Result<Intent, Reason> {
    let object = value.as_object().ok_or(Reason::Schema)?;
    Ok(Intent {
        id: req_str(object, "id")?,
        tick_index: req_u64(object, "tick_index")?,
        actor_id: req_str(object, "actor_id")?,
        actor_kind: actor_kind(&req_str(object, "actor_kind")?)?,
        source: intent_source(&req_str(object, "source")?)?,
        move_name: req_str(object, "move")?,
        target: parse_target(object.get("target").ok_or(Reason::Schema)?)?,
        args: parse_args(object.get("args").ok_or(Reason::Schema)?)?,
        rationale_code: req_str(object, "rationale_code")?,
    })
}

fn parse_target(value: &Value) -> Result<Target, Reason> {
    let object = value.as_object().ok_or(Reason::Schema)?;
    if object.is_empty() {
        return Ok(Target::Empty);
    }
    if object.len() != 1 {
        return Err(Reason::Schema);
    }
    if let Some(place) = object.get("place").and_then(Value::as_str) {
        return Ok(Target::Place(place.to_string()));
    }
    if let Some(actor) = object.get("actor").and_then(Value::as_str) {
        return Ok(Target::Actor(actor.to_string()));
    }
    if let Some(clock) = object.get("clock").and_then(Value::as_str) {
        return Ok(Target::Clock(clock.to_string()));
    }
    Err(Reason::Schema)
}

fn parse_args(value: &Value) -> Result<IntentArgs, Reason> {
    let object = value.as_object().ok_or(Reason::Schema)?;
    let mut args = IntentArgs::default();
    for (key, item) in object {
        match key.as_str() {
            "to" => args.message_to = Some(req_string_array(item)?),
            _ => return Err(Reason::Schema),
        }
    }
    Ok(args)
}

fn req_string_array(value: &Value) -> Result<Vec<String>, Reason> {
    let values = value.as_array().ok_or(Reason::Schema)?;
    let mut ids = Vec::new();
    for item in values {
        ids.push(item.as_str().ok_or(Reason::Schema)?.to_string());
    }
    Ok(ids)
}

fn actor_kind(text: &str) -> Result<ActorKind, Reason> {
    match text {
        "npc" => Ok(ActorKind::Npc),
        "player" => Ok(ActorKind::Player),
        "system" => Ok(ActorKind::System),
        _ => Err(Reason::Schema),
    }
}

fn intent_source(text: &str) -> Result<Source, Reason> {
    match text {
        "kernel" => Ok(Source::Kernel),
        "player" => Ok(Source::Player),
        "system" => Ok(Source::System),
        _ => Err(Reason::Schema),
    }
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
        "meter" => {
            allow(object, &["actor_id", "key", "write", "delta"])?;
            let actor_id = req_str(object, "actor_id")?;
            let key = req_str(object, "key")?;
            let write = match object.get("write").and_then(Value::as_str) {
                Some("base") => MeterWrite::Base(req_i64(object, "delta")?),
                Some("offset") => MeterWrite::Offset(req_i64(object, "delta")?),
                Some("snap") => {
                    if object.contains_key("delta") {
                        return Err(Reason::Schema);
                    }
                    MeterWrite::Snap
                }
                _ => return Err(Reason::Schema),
            };
            Ok(EffectClass::Meter {
                actor_id,
                key,
                write,
            })
        }
        "item" => {
            allow(object, &["actor_id", "place", "id", "delta", "max"])?;
            let actor_id = opt_str(object, "actor_id")?;
            let place = opt_str(object, "place")?;
            if actor_id.is_some() == place.is_some() {
                return Err(Reason::Schema);
            }
            Ok(EffectClass::Item {
                actor_id,
                place,
                id: req_str(object, "id")?,
                delta: req_i64(object, "delta")?,
                max: opt_i64(object, "max")?,
            })
        }
        "wield" => {
            allow(object, &["actor_id", "id"])?;
            Ok(EffectClass::Wield {
                actor_id: req_str(object, "actor_id")?,
                id: req_str(object, "id")?,
            })
        }
        "unwield" => {
            allow(object, &["actor_id"])?;
            Ok(EffectClass::Unwield {
                actor_id: req_str(object, "actor_id")?,
            })
        }
        "narrate" => {
            allow(object, &["to", "fact_ids", "text"])?;
            Ok(EffectClass::Narrate {
                to: req_strs(object, "to")?,
                fact_ids: req_strs(object, "fact_ids")?,
                text: req_str(object, "text")?,
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
        "memory" => {
            allow(object, &["actor_id", "fact_ids", "tags", "text"])?;
            Ok(EffectClass::Memory {
                actor_id: req_str(object, "actor_id")?,
                fact_ids: req_strs(object, "fact_ids")?,
                tags: req_strs(object, "tags")?,
                text: opt_str(object, "text")?,
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
        _ => Err(Reason::IllegalClass),
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

fn req_u64(object: &Map<String, Value>, key: &str) -> Result<u64, Reason> {
    object
        .get(key)
        .and_then(Value::as_u64)
        .ok_or(Reason::Schema)
}

fn opt_i64(object: &Map<String, Value>, key: &str) -> Result<Option<i64>, Reason> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value.as_i64().map(Some).ok_or(Reason::Schema),
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
        assert_eq!(emitted, original);
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

    use crate::effect::EffectClass;
    use crate::error::Reason;
    use crate::intent::ViewerId;
    use crate::model::MemoryLine;

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
                    EffectClass::Meter {
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
                    EffectClass::Item {
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
                    EffectClass::Meter {
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
        for name in ["need", "resource", "inventory"] {
            let value = serde_json::json!({
                "class": name,
                "as": "npc:mira",
                "intent_id": "i",
            });
            assert_eq!(
                super::parse_effect(&value).unwrap_err(),
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
            super::parse_effect(&snap_delta).unwrap_err(),
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
            super::parse_effect(&missing_delta).unwrap_err(),
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
