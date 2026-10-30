//! The GM tools: status, perceive, intent, say, commit, and close.

use serde_json::{Map, Value};
use spookery_engine::{Address, IntentCall, Presence, QueuedMove, Reason, RoundOutput, World};

use super::frames::{self, SayOutput};
use super::world_loop::commit_then_fast_forward;
use crate::protocol::SayArgs;

pub struct Reply {
    pub value: Value,
    /// Written on the GM socket after this call's response line.
    pub gm_events: Vec<Vec<u8>>,
    pub player_line: Option<(Vec<String>, Vec<u8>)>,
    pub tape_line: Option<String>,
    /// The GM seat is cleared before this reply is sent.
    pub release_seat: bool,
    /// Set when the call was refused; the host logs it.
    pub refused: Option<Reason>,
    /// Rounds this call played, commit first.
    pub outputs: Vec<RoundOutput>,
}

impl Reply {
    fn new(value: Value) -> Self {
        Self {
            value,
            gm_events: Vec::new(),
            player_line: None,
            tape_line: None,
            release_seat: false,
            refused: None,
            outputs: Vec::new(),
        }
    }
}

/// Sitting `tools/list` order. Each name is one handler.
/// Any other name is `NOT_IN_SPEC` and writes nothing.
pub(super) struct Tool {
    pub(super) name: &'static str,
    pub(super) about: &'static str,
    pub(super) schema: fn() -> Map<String, Value>,
    run: fn(&mut World, &Presence, Option<Map<String, Value>>) -> Reply,
}

const SEAT_NOTES: &str = "Player mechanical lines stay queued until commit_round. say appends speech and does not tick. perceive requires as set to an actor id. When a notification does not arrive, table_status with since returns player speech and the queue.";

impl Tool {
    pub(super) fn description(&self) -> String {
        format!("{} {}", self.about, SEAT_NOTES)
    }
}

pub(super) const TOOLS: &[Tool] = &[
    Tool {
        name: "table_status",
        about: "table_status takes no required key. An empty object stays legal. since is the only other accepted key.",
        schema: status_schema,
        run: run_table_status,
    },
    Tool {
        name: "perceive",
        about: "perceive requires as. max_body_chars may be sent. A missing as is UNSCOPED_PERCEIVE.",
        schema: perceive_schema,
        run: run_perceive,
    },
    Tool {
        name: "queue_intent",
        about: "queue_intent requires actor_id and move. A habit row is legal when actor_id is an npc.",
        schema: intent_schema,
        run: run_intent,
    },
    Tool {
        name: "say",
        about: "say requires address and text. place is required when address is place. actor is required when address is actor. table takes neither place nor actor. A supplied from is SCHEMA.",
        schema: say_schema,
        run: run_say,
    },
    Tool {
        name: "commit_round",
        about: "commit_round requires include_round. session_id may be a string.",
        schema: commit_schema,
        run: run_commit_round,
    },
    Tool {
        name: "close",
        about: "close takes no keys. The result is sent after this GM seat is free.",
        schema: close_schema,
        run: run_close,
    },
];

/// Run one GM tool. `presence` is who the host's seats hold.
pub fn call_gm_tool(
    world: &mut World,
    presence: &Presence,
    name: &str,
    arguments: Option<Map<String, Value>>,
) -> Reply {
    match TOOLS.iter().find(|tool| tool.name == name) {
        Some(tool) => (tool.run)(world, presence, arguments),
        None => refused_reply(Reason::NotInSpec),
    }
}

fn run_intent(
    world: &mut World,
    _presence: &Presence,
    arguments: Option<Map<String, Value>>,
) -> Reply {
    match queue_tool_intent(world, arguments) {
        Ok(queued) => Reply::new(crate::protocol::queued_result(&queued.id)),
        Err(code) => refused_reply(code),
    }
}

fn run_commit_round(
    world: &mut World,
    presence: &Presence,
    arguments: Option<Map<String, Value>>,
) -> Reply {
    match try_commit_round(world, presence, arguments) {
        Ok(done) => done,
        Err(code) => refused_reply(code),
    }
}

fn run_perceive(
    world: &mut World,
    presence: &Presence,
    arguments: Option<Map<String, Value>>,
) -> Reply {
    match try_perceive(world, presence, arguments) {
        Ok(value) => Reply::new(value),
        Err(code) => refused_reply(code),
    }
}

fn run_say(world: &mut World, presence: &Presence, arguments: Option<Map<String, Value>>) -> Reply {
    match try_say(world, presence, arguments) {
        Ok(spoken) => {
            let mut done = Reply::new(spoken.tool_result);
            // Same object as the tool result, after that response. Ahead of it, the client stalls.
            done.gm_events.push(spoken.gm_event);
            done.player_line = Some((spoken.recipients, spoken.player_line.into_bytes()));
            done.tape_line = Some(spoken.tape_line);
            done
        }
        Err(code) => refused_reply(code),
    }
}

/// GM `say`. `from` is the seat. A supplied `from` appends nothing.
fn try_say(
    world: &World,
    presence: &Presence,
    arguments: Option<Map<String, Value>>,
) -> Result<SayOutput, Reason> {
    let say = crate::protocol::parse_say_args(arguments.as_ref()).map_err(|_| Reason::Schema)?;
    let heard = match &say {
        SayArgs::Table { .. } => Address::Table,
        SayArgs::Place { place, .. } => Address::Place(place),
        SayArgs::Actor { actor, .. } => Address::Actor(actor),
    };
    let stored = world.say_as_gm(presence, heard, say.text())?;
    Ok(SayOutput::from(&stored))
}

fn run_table_status(
    world: &mut World,
    presence: &Presence,
    arguments: Option<Map<String, Value>>,
) -> Reply {
    match try_table_status(world, presence, arguments) {
        Ok(value) => Reply::new(value),
        Err(code) => refused_reply(code),
    }
}

fn run_close(
    _world: &mut World,
    _presence: &Presence,
    arguments: Option<Map<String, Value>>,
) -> Reply {
    if arguments.is_some_and(|args| !args.is_empty()) {
        return refused_reply(Reason::Schema);
    }
    let mut done = Reply::new(crate::protocol::released_result());
    done.release_seat = true;
    done
}

fn say_schema() -> Map<String, Value> {
    schema_object(serde_json::json!({
        "type": "object",
        "properties": {
            "address": { "type": "string", "enum": ["table", "place", "actor"] },
            "text": { "type": "string" },
            "place": { "type": ["string", "null"] },
            "actor": { "type": ["string", "null"] }
        },
        "required": ["address", "text"],
        "additionalProperties": false,
        "allOf": [
            {
                "if": {
                    "properties": { "address": { "const": "place" } },
                    "required": ["address"]
                },
                "then": {
                    "required": ["place"],
                    "properties": {
                        "place": { "type": "string", "minLength": 1 },
                        "actor": { "type": "null" }
                    }
                }
            },
            {
                "if": {
                    "properties": { "address": { "const": "actor" } },
                    "required": ["address"]
                },
                "then": {
                    "required": ["actor"],
                    "properties": {
                        "actor": { "type": "string", "minLength": 1 },
                        "place": { "type": "null" }
                    }
                }
            },
            {
                "if": {
                    "properties": { "address": { "const": "table" } },
                    "required": ["address"]
                },
                "then": {
                    "properties": {
                        "place": { "type": "null" },
                        "actor": { "type": "null" }
                    }
                }
            }
        ]
    }))
}

fn perceive_schema() -> Map<String, Value> {
    schema_object(serde_json::json!({
        "type": "object",
        "properties": {
            "as": { "type": "string" },
            "max_body_chars": { "type": "integer", "minimum": 0 }
        },
        "required": ["as"],
        "additionalProperties": false
    }))
}

fn commit_schema() -> Map<String, Value> {
    schema_object(serde_json::json!({
        "type": "object",
        "properties": {
            "include_round": { "type": "boolean" },
            "session_id": { "type": "string" }
        },
        "required": ["include_round"],
        "additionalProperties": false
    }))
}

fn close_schema() -> Map<String, Value> {
    schema_object(serde_json::json!({
        "type": "object",
        "properties": {},
        "additionalProperties": false
    }))
}

fn status_schema() -> Map<String, Value> {
    schema_object(serde_json::json!({
        "type": "object",
        "properties": {
            "since": { "type": "integer", "minimum": 0 }
        },
        "additionalProperties": false
    }))
}

fn intent_schema() -> Map<String, Value> {
    schema_object(serde_json::json!({
        "type": "object",
        "properties": {
            "actor_id": { "type": "string" },
            "move": { "type": "string" },
            "id": { "type": "string" },
            "round_index": { "type": "integer", "minimum": 0 },
            "target": { "type": "string" },
            "to": { "type": "array", "items": { "type": "string" } },
            "rationale_code": { "type": "string" }
        },
        "required": ["actor_id", "move"],
        "additionalProperties": false
    }))
}

fn schema_object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

fn try_commit_round(
    world: &mut World,
    presence: &Presence,
    arguments: Option<Map<String, Value>>,
) -> Result<Reply, Reason> {
    let include_round = read_include_round(arguments)?;
    if !include_round {
        if world.queued_intents().is_empty() {
            return Ok(empty_commit(world));
        }
        return Err(Reason::NotInSpec);
    }
    let committed = !world.queued_intents().is_empty();
    let outputs = commit_then_fast_forward(world, presence)?;
    let mut done = if committed {
        let output = &outputs[0];
        let outcome = output
            .commit
            .as_ref()
            .expect("a flush round writes the journal");
        Reply::new(frames::commit_reply(
            outcome.state_rev,
            &output.summary,
            Some(&outcome.log_path),
            &outputs,
        ))
    } else if outputs.is_empty() {
        return Ok(empty_commit(world));
    } else {
        Reply::new(frames::commit_reply(
            world.state().state_rev,
            &[],
            None,
            &outputs,
        ))
    };
    done.outputs = outputs;
    Ok(done)
}

fn empty_commit(world: &World) -> Reply {
    Reply::new(frames::commit_reply(
        world.state().state_rev,
        &[],
        None,
        &[],
    ))
}

fn refused_reply(code: Reason) -> Reply {
    let mut done = Reply::new(crate::protocol::code_result(code.as_str()));
    done.refused = Some(code);
    done
}

fn try_perceive(
    world: &World,
    presence: &Presence,
    arguments: Option<Map<String, Value>>,
) -> Result<Value, Reason> {
    let args =
        crate::protocol::parse_perceive_args(arguments.as_ref()).map_err(|_| Reason::Schema)?;
    let as_arg = args.as_viewer.ok_or(Reason::UnscopedPerceive)?;
    let budget = usize::try_from(args.max_body_chars.unwrap_or(800)).map_err(|_| Reason::Schema)?;
    let view = world.perceive_trimmed(presence, &as_arg, budget)?;
    Ok(frames::perceive_value(&view))
}

fn try_table_status(
    world: &World,
    presence: &Presence,
    arguments: Option<Map<String, Value>>,
) -> Result<Value, Reason> {
    let since = read_since(arguments)?;
    let events = match since {
        Some(since) => Some(world.tape_after(since)?),
        None => None,
    };
    let last_seq = world.last_tape_seq()?;
    Ok(frames::table_status(
        world.state(),
        presence,
        world.queued_intents(),
        last_seq,
        events.as_deref(),
        since.is_none().then(|| world.pack_summary()),
    ))
}

fn read_since(arguments: Option<Map<String, Value>>) -> Result<Option<u64>, Reason> {
    let args =
        crate::protocol::parse_status_args(arguments.as_ref()).map_err(|_| Reason::Schema)?;
    Ok(args.since)
}

fn read_include_round(arguments: Option<Map<String, Value>>) -> Result<bool, Reason> {
    let args =
        crate::protocol::parse_commit_args(arguments.as_ref()).map_err(|_| Reason::Schema)?;
    Ok(args.include_round)
}

fn queue_tool_intent(
    world: &mut World,
    arguments: Option<Map<String, Value>>,
) -> Result<QueuedMove, Reason> {
    let call =
        crate::protocol::parse_intent_args(arguments.as_ref()).map_err(|_| Reason::Schema)?;
    world.queue_gm_intent(
        IntentCall {
            id: call.id,
            round_index: call.round_index,
            actor_id: call.actor_id,
            move_name: call.move_name,
            target: call.target,
            message_to: call.message_to,
            rationale_code: call.rationale_code,
        },
        || Ok(uuid::Uuid::new_v4().to_string()),
    )
}
