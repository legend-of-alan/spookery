//! Player lines, tape objects, `spook/event` frames, and tool results.
//! A tape line this parser cannot read is `SCHEMA` for the whole read.

use serde_json::{Map, Value};
use spookery_kernel::check::CheckOut;
use spookery_kernel::intent::Intent;
use spookery_kernel::model::{Message, Now, RunState};
use spookery_kernel::Reason;
use spookery_kernel::perceive::{ClockView, KnownFact, Perceive, ScheduleHint, StockView};

pub struct TapeLine {
    pub seq: u64,
    pub from: String,
    pub address: String,
    pub place: Option<String>,
    pub actor: Option<String>,
    pub recipients: Vec<String>,
    pub text: String,
    pub tick_index: u64,
    pub world_time: u64,
}

pub fn parse_tape(line: &str) -> Result<TapeLine, Reason> {
    let value: Value = serde_json::from_str(line).map_err(|_| Reason::Schema)?;
    let recipients = value
        .get("recipients")
        .and_then(Value::as_array)
        .ok_or(Reason::Schema)?;
    let mut ids = Vec::new();
    for id in recipients {
        ids.push(
            id.as_str()
                .ok_or(Reason::Schema)?
                .to_string(),
        );
    }
    Ok(TapeLine {
        seq: value.get("seq").and_then(Value::as_u64).ok_or(Reason::Schema)?,
        from: value
            .get("from")
            .and_then(Value::as_str)
            .ok_or(Reason::Schema)?
            .to_string(),
        address: value
            .get("address")
            .and_then(Value::as_str)
            .ok_or(Reason::Schema)?
            .to_string(),
        place: optional_string(value.get("place").ok_or(Reason::Schema)?).ok_or(Reason::Schema)?,
        actor: optional_string(value.get("actor").ok_or(Reason::Schema)?).ok_or(Reason::Schema)?,
        recipients: ids,
        text: value
            .get("text")
            .and_then(Value::as_str)
            .ok_or(Reason::Schema)?
            .to_string(),
        tick_index: value
            .get("tick_index")
            .and_then(Value::as_u64)
            .ok_or(Reason::Schema)?,
        world_time: value
            .get("world_time")
            .and_then(Value::as_u64)
            .ok_or(Reason::Schema)?,
    })
}

pub fn tape_body(tape: &TapeLine) -> String {
    compact(&obj([
        ("seq", Value::from(tape.seq)),
        ("from", Value::String(tape.from.clone())),
        ("address", Value::String(tape.address.clone())),
        ("place", opt_str(tape.place.as_deref())),
        ("actor", opt_str(tape.actor.as_deref())),
        ("recipients", string_list(&tape.recipients)),
        ("text", Value::String(tape.text.clone())),
        ("tick_index", Value::from(tape.tick_index)),
        ("world_time", Value::from(tape.world_time)),
    ]))
}

pub fn player_say_line(tape: &TapeLine) -> String {
    ended(&obj([
        ("kind", json_str("say")),
        ("from", Value::String(tape.from.clone())),
        ("address", Value::String(tape.address.clone())),
        ("text", Value::String(tape.text.clone())),
    ]))
}

pub fn say_value(tape: &TapeLine) -> Value {
    obj([
        ("kind", json_str("say")),
        ("seq", Value::from(tape.seq)),
        ("from", Value::String(tape.from.clone())),
        ("address", Value::String(tape.address.clone())),
        ("place", opt_str(tape.place.as_deref())),
        ("actor", opt_str(tape.actor.as_deref())),
        ("recipients", string_list(&tape.recipients)),
        ("text", Value::String(tape.text.clone())),
    ])
}

pub fn say_event(tape: &TapeLine) -> Vec<u8> {
    event(say_value(tape))
}

pub fn player_queued_line(id: &str) -> String {
    ended(&obj([
        ("kind", json_str("queued")),
        ("id", Value::String(id.to_string())),
    ]))
}

pub fn queued_event(id: &str, actor_id: &str, move_name: &str) -> Vec<u8> {
    event(obj([
        ("kind", json_str("queued")),
        ("id", Value::String(id.to_string())),
        ("actor_id", Value::String(actor_id.to_string())),
        ("move", Value::String(move_name.to_string())),
    ]))
}

pub fn player_check_line(success: bool) -> String {
    ended(&obj([
        ("kind", json_str("check")),
        ("success", Value::Bool(success)),
    ]))
}

pub fn check_event(out: &CheckOut) -> Vec<u8> {
    let faces = Value::Array(out.faces.iter().copied().map(Value::from).collect());
    event(obj([
        ("kind", json_str("check")),
        ("faces", faces),
        ("stat", Value::from(out.stat)),
        ("total", Value::from(out.total)),
        ("target", Value::from(out.target)),
        ("success", Value::Bool(out.success)),
    ]))
}

pub fn player_resume_line(text: &str) -> String {
    ended(&obj([
        ("kind", json_str("resume")),
        ("text", Value::String(text.to_string())),
    ]))
}

pub fn player_refused_line(code: &str) -> String {
    ended(&obj([
        ("kind", json_str("refused")),
        ("code", Value::String(code.to_string())),
    ]))
}

pub fn player_perceive_line(view: &Perceive) -> String {
    ended(&perceive_value(view))
}

pub fn perceive_value(view: &Perceive) -> Value {
    let mut map = Map::new();
    map.insert("kind".to_string(), json_str("perceive"));
    map.insert("viewer".to_string(), Value::String(view.viewer.clone()));
    map.insert("place".to_string(), Value::String(view.place.clone()));
    map.insert(
        "visible_actors".to_string(),
        string_list(&view.visible_actors),
    );
    map.insert(
        "known_facts".to_string(),
        Value::Array(view.known_facts.iter().map(fact_value).collect()),
    );
    map.insert(
        "messages".to_string(),
        Value::Array(view.messages.iter().map(message_value).collect()),
    );
    map.insert(
        "unknown_mentioned".to_string(),
        string_list(&view.unknown_mentioned),
    );
    map.insert(
        "clocks".to_string(),
        Value::Array(view.clocks.iter().map(clock_value).collect()),
    );
    map.insert(
        "stocks".to_string(),
        Value::Array(view.stocks.iter().map(stock_value).collect()),
    );
    map.insert(
        "schedule_hint".to_string(),
        match &view.schedule_hint {
            Some(hint) => hint_value(hint),
            None => Value::Null,
        },
    );
    Value::Object(map)
}

pub fn commit_turn(state_rev: u64, diff_summary: &[String], log_path: Option<&str>) -> Value {
    obj([
        ("state_rev", Value::from(state_rev)),
        ("diff_summary", string_list(diff_summary)),
        (
            "log_path",
            match log_path {
                Some(path) => Value::String(path.to_string()),
                None => Value::Null,
            },
        ),
    ])
}

pub fn queued_ack(id: &str) -> Value {
    obj([("queued", Value::String(id.to_string()))])
}

pub fn code_result(code: &str) -> Value {
    obj([("code", Value::String(code.to_string()))])
}

pub fn session_status(
    now: &Now,
    queued: &[Intent],
    last_seq: u64,
    events: Option<&[TapeLine]>,
) -> Value {
    let mut map = Map::new();
    map.insert(
        "presence".to_string(),
        obj([
            ("humans", Value::from(now.presence.humans)),
            ("actors_present", string_list(&now.presence.actors_present)),
        ]),
    );
    map.insert(
        "pause".to_string(),
        obj([
            ("named_plot_clocks", run_state(now.pause.named_plot_clocks)),
            (
                "relationship_drift",
                run_state(now.pause.relationship_drift),
            ),
            (
                "subsistence_clocks",
                run_state(now.pause.subsistence_clocks),
            ),
            ("travel", run_state(now.pause.travel)),
        ]),
    );
    map.insert("world_time".to_string(), Value::from(now.world_time));
    map.insert("tick_index".to_string(), Value::from(now.tick_index));
    map.insert("state_rev".to_string(), Value::from(now.state_rev));
    map.insert("pack_rev".to_string(), Value::String(now.pack_rev.clone()));
    map.insert(
        "last_tick_at".to_string(),
        Value::String(now.last_tick_at.clone()),
    );
    map.insert(
        "queued".to_string(),
        Value::Array(
            queued
                .iter()
                .map(|intent| {
                    obj([
                        ("id", Value::String(intent.id.clone())),
                        ("actor_id", Value::String(intent.actor_id.clone())),
                        ("move", Value::String(intent.move_name.clone())),
                    ])
                })
                .collect(),
        ),
    );
    map.insert("last_seq".to_string(), Value::from(last_seq));
    if let Some(events) = events {
        map.insert(
            "events".to_string(),
            Value::Array(events.iter().map(say_value).collect()),
        );
    }
    Value::Object(map)
}

fn fact_value(fact: &KnownFact) -> Value {
    obj([
        ("id", Value::String(fact.id.clone())),
        ("text", Value::String(fact.text.clone())),
        ("body", Value::String(fact.body.clone())),
    ])
}

fn message_value(message: &Message) -> Value {
    let mut map = Map::new();
    map.insert("id".to_string(), Value::String(message.id.clone()));
    map.insert("to".to_string(), string_list(&message.to));
    if let Some(place) = &message.place {
        map.insert("place".to_string(), Value::String(place.clone()));
    }
    map.insert("fact_ids".to_string(), string_list(&message.fact_ids));
    map.insert("text".to_string(), Value::String(message.text.clone()));
    Value::Object(map)
}

fn clock_value(clock: &ClockView) -> Value {
    obj([
        ("id", Value::String(clock.id.clone())),
        ("value", Value::from(clock.value)),
        ("max", Value::from(clock.max)),
        ("paused", Value::Bool(clock.paused)),
    ])
}

fn stock_value(stock: &StockView) -> Value {
    obj([
        ("sku", Value::String(stock.sku.clone())),
        ("qty", Value::from(stock.qty)),
    ])
}

fn hint_value(hint: &ScheduleHint) -> Value {
    obj([
        ("place", Value::String(hint.place.clone())),
        (
            "activity",
            match &hint.activity {
                Some(activity) => Value::String(activity.clone()),
                None => Value::Null,
            },
        ),
    ])
}

fn run_state(state: RunState) -> Value {
    Value::String(
        match state {
            RunState::Paused => "paused",
            RunState::Running => "running",
        }
        .to_string(),
    )
}

fn event(params: Value) -> Vec<u8> {
    let mut line = compact(&obj([
        ("jsonrpc", json_str("2.0")),
        ("method", json_str("spook/event")),
        ("params", params),
    ]));
    line.push('\n');
    line.into_bytes()
}

fn obj<const N: usize>(entries: [(&str, Value); N]) -> Value {
    let mut map = Map::new();
    for (key, value) in entries {
        map.insert(key.to_string(), value);
    }
    Value::Object(map)
}

fn string_list(values: &[String]) -> Value {
    Value::Array(values.iter().cloned().map(Value::String).collect())
}

fn opt_str(value: Option<&str>) -> Value {
    match value {
        Some(text) => Value::String(text.to_string()),
        None => Value::Null,
    }
}

fn json_str(text: &str) -> Value {
    Value::String(text.to_string())
}

fn optional_string(value: &Value) -> Option<Option<String>> {
    if value.is_null() {
        Some(None)
    } else {
        Some(Some(value.as_str()?.to_string()))
    }
}

fn compact(value: &Value) -> String {
    serde_json::to_string(value).expect("wire json")
}

fn ended(value: &Value) -> String {
    let mut line = compact(value);
    line.push('\n');
    line
}
