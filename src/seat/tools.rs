use serde_json::{Map, Value};
use spookery_kernel::intent::{ActorKind, IntentArgs, Target};
use spookery_kernel::model::ActorLive;
use spookery_kernel::moves::Audience;
use spookery_kernel::perceive::perceive_budget;
use spookery_kernel::shard::Shard;
use spookery_kernel::Reason;
use spookery_tick::commit_turn_at;

use super::protocol;
use super::speech;
use super::turn::{self, QueuedNote};
use crate::schedule::wall_now;

pub struct ToolResult {
    pub value: Value,
    pub advanced: bool,
    pub ticked: bool,
    pub gm_event: Option<Vec<u8>>,
    /// Written on the GM socket after this call's response line.
    pub after: Vec<Vec<u8>>,
    pub player_line: Option<(Vec<String>, Vec<u8>)>,
    pub tape: Option<String>,
}

impl ToolResult {
    fn new(value: Value) -> Self {
        Self {
            value,
            advanced: false,
            ticked: false,
            gm_event: None,
            after: Vec::new(),
            player_line: None,
            tape: None,
        }
    }
}

/// Sitting `tools/list` order. Each name is one handler.
/// Any other name is `NOT_IN_SPEC` and writes nothing.
pub(super) struct GmTool {
    pub(super) name: &'static str,
    pub(super) about: &'static str,
    pub(super) schema: fn() -> Map<String, Value>,
    run: fn(&mut Shard, Option<Map<String, Value>>) -> ToolResult,
}

const SEAT_FACTS: &str = "Player mechanical lines stay queued until commit_turn. say appends speech and does not tick. perceive requires as set to player:<id> or npc:<id>. This seat does not call session_open or patch_entity. When a notification does not arrive, session_status with since returns player speech and the queue.";

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
        name: "commit_turn",
        about: "commit_turn requires include_tick. session_id may be a string.",
        schema: commit_schema,
        run: run_commit_turn,
    },
];

pub fn gm_tool(shard: &mut Shard, name: &str, arguments: Option<Map<String, Value>>) -> ToolResult {
    match GM_LIST.iter().find(|tool| tool.name == name) {
        Some(tool) => (tool.run)(shard, arguments),
        None => refuse_not_in_spec(shard, arguments),
    }
}

fn run_intent(shard: &mut Shard, arguments: Option<Map<String, Value>>) -> ToolResult {
    match queue_tool_intent(shard, arguments) {
        Ok(note) => ToolResult::new(protocol::queued_ack(&note.id)),
        Err(code) => refused_tool(code),
    }
}

fn run_commit_turn(shard: &mut Shard, arguments: Option<Map<String, Value>>) -> ToolResult {
    match commit_tool(shard, arguments) {
        Ok(done) => done,
        Err(code) => refused_tool(code),
    }
}

fn run_perceive(shard: &mut Shard, arguments: Option<Map<String, Value>>) -> ToolResult {
    match perceive_tool(shard, arguments) {
        Ok(value) => ToolResult::new(value),
        Err(code) => refused_tool(code),
    }
}

fn run_say(shard: &mut Shard, arguments: Option<Map<String, Value>>) -> ToolResult {
    match speech::speak_tool(shard, arguments) {
        Ok(spoken) => {
            let mut done = ToolResult::new(spoken.tool);
            // Same object as the tool result, after that response. Ahead of it, the client stalls.
            done.after.push(spoken.gm_event);
            done.player_line = Some((spoken.recipients, spoken.player_event.into_bytes()));
            done.tape = Some(spoken.tape);
            done
        }
        Err(code) => refused_tool(code),
    }
}

fn run_session_status(shard: &mut Shard, arguments: Option<Map<String, Value>>) -> ToolResult {
    match session_status_tool(shard, arguments) {
        Ok(value) => ToolResult::new(value),
        Err(code) => refused_tool(code),
    }
}

fn refuse_not_in_spec(_shard: &mut Shard, _arguments: Option<Map<String, Value>>) -> ToolResult {
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
    shard: &mut Shard,
    arguments: Option<Map<String, Value>>,
) -> Result<ToolResult, Reason> {
    let include_tick = commit_include_tick(arguments)?;
    if !include_tick {
        if shard.queued_intents().is_empty() {
            return Ok(noop(shard));
        }
        return Err(Reason::NotInSpec);
    }
    if shard.queued_intents().is_empty() {
        return Ok(noop(shard));
    }
    let before = shard.now().world_time;
    let wall = wall_now()?;
    let outcome = commit_turn_at(shard, &wall)?;
    let advanced = shard.now().world_time != before;
    let mut done = ToolResult::new(protocol::commit_turn(
        outcome.state_rev,
        &outcome.diff_summary,
        Some(&outcome.log_path),
    ));
    done.advanced = advanced;
    done.ticked = true;
    Ok(done)
}

fn noop(shard: &Shard) -> ToolResult {
    ToolResult::new(protocol::commit_turn(shard.now().state_rev, &[], None))
}

fn refused_tool(code: Reason) -> ToolResult {
    ToolResult::new(protocol::code_result(code.token()))
}

fn perceive_tool(shard: &Shard, arguments: Option<Map<String, Value>>) -> Result<Value, Reason> {
    let args =
        spookery_protocol::read_perceive_args(arguments.as_ref()).map_err(|_| Reason::Schema)?;
    let as_arg = args.as_viewer.ok_or(Reason::UnscopedPerceive)?;
    let budget = usize::try_from(args.budget_tokens.unwrap_or(800)).map_err(|_| Reason::Schema)?;
    let view = perceive_budget(shard, &as_arg, budget)?;
    Ok(protocol::perceive_value(&view))
}

fn session_status_tool(
    shard: &Shard,
    arguments: Option<Map<String, Value>>,
) -> Result<Value, Reason> {
    let since = status_since(arguments)?;
    let events = match since {
        Some(since) => Some(speech::tape_after(shard.root(), since)?),
        None => None,
    };
    let last_seq = speech::last_tape_seq(shard.root())?;
    Ok(protocol::session_status(
        shard.now(),
        shard.queued_intents(),
        last_seq,
        events.as_deref(),
    ))
}

fn status_since(arguments: Option<Map<String, Value>>) -> Result<Option<u64>, Reason> {
    let args =
        spookery_protocol::read_status_args(arguments.as_ref()).map_err(|_| Reason::Schema)?;
    Ok(args.since)
}

fn commit_include_tick(arguments: Option<Map<String, Value>>) -> Result<bool, Reason> {
    let args =
        spookery_protocol::read_commit_args(arguments.as_ref()).map_err(|_| Reason::Schema)?;
    Ok(args.include_tick)
}

fn queue_tool_intent(
    shard: &mut Shard,
    arguments: Option<Map<String, Value>>,
) -> Result<QueuedNote, Reason> {
    let call =
        spookery_protocol::read_intent_args(arguments.as_ref()).map_err(|_| Reason::Schema)?;
    let id = call.id;
    let tick_index = call.tick_index;
    let actor_id = call.actor_id;
    let actor_kind = call.actor_kind.as_deref().map(parse_kind).transpose()?;
    let source = call.source;
    let move_name = call.move_name;
    let target = kernel_target(call.target);
    let args = IntentArgs {
        message_to: call.message_to,
    };
    let rationale = call.rationale_code;
    if source.as_deref() != Some("player") {
        return Err(Reason::Schema);
    }
    let actor_id = actor_id.ok_or(Reason::Schema)?;
    let actor_kind = actor_kind.ok_or(Reason::Schema)?;
    let move_name = move_name.ok_or(Reason::Schema)?;
    let audience = match shard
        .canon()
        .moves
        .get(&move_name)
        .and_then(|def| def.row.as_ref())
    {
        Some(row) => row.audience,
        None if shard.canon().moves.contains_key(&move_name) => return Err(Reason::NotInSpec),
        None => return Err(Reason::UnknownMove),
    };
    match actor_kind {
        ActorKind::Player => {
            if audience != Some(Audience::Player) {
                return Err(Reason::NotInSpec);
            }
            if !shard
                .now()
                .actors
                .get(&actor_id)
                .is_some_and(ActorLive::is_pc)
            {
                return Err(Reason::Schema);
            }
        }
        ActorKind::Npc => {
            if audience != Some(Audience::Player) && audience != Some(Audience::Habit) {
                return Err(Reason::NotInSpec);
            }
            if !shard
                .now()
                .actors
                .get(&actor_id)
                .is_some_and(ActorLive::is_npc)
            {
                return Err(Reason::Schema);
            }
        }
        ActorKind::System => return Err(Reason::Schema),
    }
    let id = match id {
        Some(id) if !id.is_empty() => id,
        Some(_) => return Err(Reason::Schema),
        None => turn::mint_intent_id()?,
    };
    if shard.queued_intents().iter().any(|intent| intent.id == id) {
        return Err(Reason::Schema);
    }
    turn::push_intent(
        shard,
        id.clone(),
        &actor_id,
        actor_kind,
        &move_name,
        target,
        args,
        rationale,
        tick_index,
    )?;
    Ok(QueuedNote {
        id,
        actor_id,
        move_name,
    })
}

fn parse_kind(text: &str) -> Result<ActorKind, Reason> {
    match text {
        "npc" => Ok(ActorKind::Npc),
        "player" => Ok(ActorKind::Player),
        "system" => Ok(ActorKind::System),
        _ => Err(Reason::Schema),
    }
}

fn kernel_target(target: spookery_protocol::CallTarget) -> Target {
    match target {
        spookery_protocol::CallTarget::Empty => Target::Empty,
        spookery_protocol::CallTarget::Place(id) => Target::Place(id),
        spookery_protocol::CallTarget::Actor(id) => Target::Actor(id),
        spookery_protocol::CallTarget::Clock(id) => Target::Clock(id),
        spookery_protocol::CallTarget::Rumor(id) => Target::Rumor(id),
    }
}
