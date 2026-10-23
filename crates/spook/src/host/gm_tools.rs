//! The GM tools: status, perceive, intent, say, commit, and close.

use serde_json::{Map, Value};
use spookery_engine::{GmIntentSpec, QueuedMove, Reason, RoundOutput, Target, World};

use super::frames;
use super::world_loop::commit_then_fast_forward;

pub struct GmToolReply {
    pub value: Value,
    pub advanced: bool,
    pub ticked: bool,
    pub gm_event: Option<Vec<u8>>,
    /// Written on the GM socket after this call's response line.
    pub after: Vec<Vec<u8>>,
    pub player_line: Option<(Vec<String>, Vec<u8>)>,
    pub tape: Option<String>,
    /// The GM seat is cleared before this reply is sent.
    pub release_seat: bool,
    /// Set when the call was refused; the host logs it.
    pub refused: Option<Reason>,
    /// Rounds this call played, commit first.
    pub outputs: Vec<RoundOutput>,
}

impl GmToolReply {
    fn new(value: Value) -> Self {
        Self {
            value,
            advanced: false,
            ticked: false,
            gm_event: None,
            after: Vec::new(),
            player_line: None,
            tape: None,
            release_seat: false,
            refused: None,
            outputs: Vec::new(),
        }
    }
}

/// Sitting `tools/list` order. Each name is one handler.
/// Any other name is `NOT_IN_SPEC` and writes nothing.
pub(super) struct GmTool {
    pub(super) name: &'static str,
    pub(super) about: &'static str,
    pub(super) schema: fn() -> Map<String, Value>,
    run: fn(&mut World, Option<Map<String, Value>>) -> GmToolReply,
}

const SEAT_FACTS: &str = "Player mechanical lines stay queued until commit_round. say appends speech and does not tick. perceive requires as set to player:<id> or npc:<id>. This seat does not call session_open or patch_entity. When a notification does not arrive, session_status with since returns player speech and the queue.";

impl GmTool {
    pub(super) fn description(&self) -> String {
        format!("{} {}", self.about, SEAT_FACTS)
    }
}

pub(super) const GM_LIST: &[GmTool] = &[
    GmTool {
        name: "session_status",
        about: "session_status takes no required key. An empty object stays legal. since is the only other accepted key.",
        schema: status_schema,
        run: run_session_status,
    },
    GmTool {
        name: "perceive",
        about: "perceive requires as. focus and budget_tokens may be sent. A missing as is UNSCOPED_PERCEIVE.",
        schema: perceive_schema,
        run: run_perceive,
    },
    GmTool {
        name: "intent",
        about: "intent requires actor_id, actor_kind, source, and move. source stays player. A habit row is legal when actor_kind is npc.",
        schema: intent_schema,
        run: run_intent,
    },
    GmTool {
        name: "say",
        about: "say requires address and text. place is required when address is place. actor is required when address is actor. table takes neither place nor actor. A supplied from is SCHEMA.",
        schema: say_schema,
        run: run_say,
    },
    GmTool {
        name: "commit_round",
        about: "commit_round requires include_tick. session_id may be a string.",
        schema: commit_schema,
        run: run_commit_round,
    },
    GmTool {
        name: "close",
        about: "close takes no keys. The result is sent after this GM seat is free.",
        schema: close_schema,
        run: run_close,
    },
];

pub fn call_gm_tool(
    world: &mut World,
    name: &str,
    arguments: Option<Map<String, Value>>,
) -> GmToolReply {
    match GM_LIST.iter().find(|tool| tool.name == name) {
        Some(tool) => (tool.run)(world, arguments),
        None => refuse_not_in_spec(world, arguments),
    }
}

fn run_intent(world: &mut World, arguments: Option<Map<String, Value>>) -> GmToolReply {
    match queue_tool_intent(world, arguments) {
        Ok(queued) => GmToolReply::new(crate::protocol::queued_ack(&queued.id)),
        Err(code) => refused_tool(code),
    }
}

fn run_commit_round(world: &mut World, arguments: Option<Map<String, Value>>) -> GmToolReply {
    match commit_tool(world, arguments) {
        Ok(done) => done,
        Err(code) => refused_tool(code),
    }
}

fn run_perceive(world: &mut World, arguments: Option<Map<String, Value>>) -> GmToolReply {
    match perceive_tool(world, arguments) {
        Ok(value) => GmToolReply::new(value),
        Err(code) => refused_tool(code),
    }
}

fn run_say(world: &mut World, arguments: Option<Map<String, Value>>) -> GmToolReply {
    match frames::speak_tool(world, arguments) {
        Ok(spoken) => {
            let mut done = GmToolReply::new(spoken.tool);
            // Same object as the tool result, after that response. Ahead of it, the client stalls.
            done.after.push(spoken.gm_event);
            done.player_line = Some((spoken.recipients, spoken.player_event.into_bytes()));
            done.tape = Some(spoken.tape);
            done
        }
        Err(code) => refused_tool(code),
    }
}

fn run_session_status(world: &mut World, arguments: Option<Map<String, Value>>) -> GmToolReply {
    match session_status_tool(world, arguments) {
        Ok(value) => GmToolReply::new(value),
        Err(code) => refused_tool(code),
    }
}

fn run_close(_world: &mut World, arguments: Option<Map<String, Value>>) -> GmToolReply {
    if arguments.is_some_and(|args| !args.is_empty()) {
        return refused_tool(Reason::Schema);
    }
    let mut done = GmToolReply::new(crate::protocol::released_result());
    done.release_seat = true;
    done
}

fn refuse_not_in_spec(_world: &mut World, _arguments: Option<Map<String, Value>>) -> GmToolReply {
    refused_tool(Reason::NotInSpec)
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
            "focus": { "type": "string", "enum": ["place", "actor"] },
            "budget_tokens": { "type": "integer", "minimum": 0 }
        },
        "required": ["as"],
        "additionalProperties": false
    }))
}

fn commit_schema() -> Map<String, Value> {
    schema_object(serde_json::json!({
        "type": "object",
        "properties": {
            "include_tick": { "type": "boolean" },
            "session_id": { "type": "string" }
        },
        "required": ["include_tick"],
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
            "actor_kind": { "type": "string", "enum": ["npc", "player"] },
            "source": { "type": "string", "const": "player" },
            "move": { "type": "string" },
            "id": { "type": "string" },
            "tick_index": { "type": "integer", "minimum": 0 },
            "target": { "type": "object" },
            "args": { "type": "object" },
            "rationale_code": { "type": "string" }
        },
        "required": ["actor_id", "actor_kind", "source", "move"],
        "additionalProperties": false
    }))
}

fn schema_object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

fn commit_tool(
    world: &mut World,
    arguments: Option<Map<String, Value>>,
) -> Result<GmToolReply, Reason> {
    let include_tick = commit_include_tick(arguments)?;
    if !include_tick {
        if world.queued_intents().is_empty() {
            return Ok(noop(world));
        }
        return Err(Reason::NotInSpec);
    }
    let before = world.now().world_time;
    let committed = !world.queued_intents().is_empty();
    let outputs = commit_then_fast_forward(world)?;
    let mut done = if committed {
        let output = &outputs[0];
        let outcome = output
            .outcome
            .as_ref()
            .expect("a flush round writes the journal");
        GmToolReply::new(frames::commit_reply(
            outcome.state_rev,
            &output.summary,
            Some(&outcome.log_path),
            &outputs,
        ))
    } else if outputs.is_empty() {
        return Ok(noop(world));
    } else {
        GmToolReply::new(frames::commit_reply(
            world.now().state_rev,
            &[],
            None,
            &outputs,
        ))
    };
    done.outputs = outputs;
    done.advanced = world.now().world_time != before;
    done.ticked = true;
    Ok(done)
}

fn noop(world: &World) -> GmToolReply {
    GmToolReply::new(frames::commit_reply(world.now().state_rev, &[], None, &[]))
}

fn refused_tool(code: Reason) -> GmToolReply {
    let mut done = GmToolReply::new(crate::protocol::code_result(code.token()));
    done.refused = Some(code);
    done
}

fn perceive_tool(world: &World, arguments: Option<Map<String, Value>>) -> Result<Value, Reason> {
    let args =
        crate::protocol::read_perceive_args(arguments.as_ref()).map_err(|_| Reason::Schema)?;
    let as_arg = args.as_viewer.ok_or(Reason::UnscopedPerceive)?;
    let budget = usize::try_from(args.budget_tokens.unwrap_or(800)).map_err(|_| Reason::Schema)?;
    let view = world.perceive_budget(&as_arg, budget)?;
    Ok(frames::perceive_value(&view))
}

fn session_status_tool(
    world: &World,
    arguments: Option<Map<String, Value>>,
) -> Result<Value, Reason> {
    let since = status_since(arguments)?;
    let events = match since {
        Some(since) => Some(world.tape_after(since)?),
        None => None,
    };
    let last_seq = world.last_tape_seq()?;
    Ok(frames::session_status(
        world.now(),
        world.queued_intents(),
        last_seq,
        events.as_deref(),
    ))
}

fn status_since(arguments: Option<Map<String, Value>>) -> Result<Option<u64>, Reason> {
    let args = crate::protocol::read_status_args(arguments.as_ref()).map_err(|_| Reason::Schema)?;
    Ok(args.since)
}

fn commit_include_tick(arguments: Option<Map<String, Value>>) -> Result<bool, Reason> {
    let args = crate::protocol::read_commit_args(arguments.as_ref()).map_err(|_| Reason::Schema)?;
    Ok(args.include_tick)
}

fn queue_tool_intent(
    world: &mut World,
    arguments: Option<Map<String, Value>>,
) -> Result<QueuedMove, Reason> {
    let call = crate::protocol::read_intent_args(arguments.as_ref()).map_err(|_| Reason::Schema)?;
    world.queue_gm_intent(
        GmIntentSpec {
            id: call.id,
            tick_index: call.tick_index,
            actor_id: call.actor_id,
            actor_kind: call.actor_kind.as_deref(),
            source: call.source.as_deref(),
            move_name: call.move_name,
            target: kernel_target(call.target),
            message_to: call.message_to,
            rationale_code: call.rationale_code,
        },
        mint_intent_id,
    )
}

fn kernel_target(target: crate::protocol::CallTarget) -> Target {
    match target {
        crate::protocol::CallTarget::Empty => Target::Empty,
        crate::protocol::CallTarget::Place(id) => Target::Place(id),
        crate::protocol::CallTarget::Actor(id) => Target::Actor(id),
        crate::protocol::CallTarget::Clock(id) => Target::Clock(id),
    }
}

fn mint_intent_id() -> Result<String, Reason> {
    let mut bytes = [0u8; 16];
    if !fill_random(&mut bytes) {
        return Err(Reason::Schema);
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Ok(format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    ))
}

#[cfg(windows)]
fn fill_random(buf: &mut [u8]) -> bool {
    #[link(name = "advapi32")]
    extern "system" {
        fn SystemFunction036(buf: *mut u8, len: u32) -> u8;
    }
    unsafe { SystemFunction036(buf.as_mut_ptr(), buf.len() as u32) != 0 }
}

#[cfg(unix)]
fn fill_random(buf: &mut [u8]) -> bool {
    use std::io::Read;
    std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(buf))
        .is_ok()
}

#[cfg(not(any(windows, unix)))]
fn fill_random(_buf: &mut [u8]) -> bool {
    false
}
