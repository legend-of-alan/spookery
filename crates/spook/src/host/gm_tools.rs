//! The GM tools: status, perceive, intent, say, commit, and close.

use serde_json::{Map, Value};
use spookery_kernel::api::intent::Intent;
use spookery_kernel::api::intent::{ActorKind, IntentArgs, Source, Target};
use spookery_kernel::api::model::ActorLive;
use spookery_kernel::api::perceive::perceive_budget;
use spookery_kernel::api::rules::gate::resolve;
use spookery_kernel::api::world::World;
use spookery_kernel::api::Reason;
use spookery_round::RoundOutput;

use super::frames;
use super::player_moves::{self, QueuedNote};
use super::speech;

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
        Ok(note) => GmToolReply::new(frames::queued_ack(&note.id)),
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
    match speech::speak_tool(world, arguments) {
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
    let mut done = GmToolReply::new(frames::released_result());
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
    let forwarded = player_moves::commit_then_fast_forward(world)?;
    let outputs = forwarded.outputs;
    let mut done = if forwarded.committed {
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
    let mut done = GmToolReply::new(frames::code_result(code.token()));
    done.refused = Some(code);
    done
}

fn perceive_tool(world: &World, arguments: Option<Map<String, Value>>) -> Result<Value, Reason> {
    let args =
        crate::protocol::read_perceive_args(arguments.as_ref()).map_err(|_| Reason::Schema)?;
    let as_arg = args.as_viewer.ok_or(Reason::UnscopedPerceive)?;
    let budget = usize::try_from(args.budget_tokens.unwrap_or(800)).map_err(|_| Reason::Schema)?;
    let view = perceive_budget(world, &as_arg, budget)?;
    Ok(frames::perceive_value(&view))
}

fn session_status_tool(
    world: &World,
    arguments: Option<Map<String, Value>>,
) -> Result<Value, Reason> {
    let since = status_since(arguments)?;
    let events = match since {
        Some(since) => Some(speech::tape_after(world.root(), since)?),
        None => None,
    };
    let last_seq = speech::last_tape_seq(world.root())?;
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
) -> Result<QueuedNote, Reason> {
    let call = crate::protocol::read_intent_args(arguments.as_ref()).map_err(|_| Reason::Schema)?;
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
    match actor_kind {
        ActorKind::Player => {
            if !world
                .now()
                .actors
                .get(&actor_id)
                .is_some_and(ActorLive::is_pc)
            {
                return Err(Reason::Schema);
            }
        }
        ActorKind::Npc => {
            if !world
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
    let probe = Intent {
        id: "probe".to_string(),
        tick_index: world.now().tick_index,
        actor_id: actor_id.clone(),
        actor_kind,
        source: Source::Player,
        move_name: move_name.clone(),
        target: target.clone(),
        args: args.clone(),
        rationale_code: String::new(),
    };
    resolve(world.canon(), world.now(), &probe)?;
    let id = match id {
        Some(id) if !id.is_empty() => id,
        Some(_) => return Err(Reason::Schema),
        None => player_moves::mint_intent_id()?,
    };
    if world.queued_intents().iter().any(|intent| intent.id == id) {
        return Err(Reason::Schema);
    }
    player_moves::push_intent(
        world,
        player_moves::IntentPush {
            id: id.clone(),
            actor_id: &actor_id,
            actor_kind,
            move_name: &move_name,
            target,
            args,
            rationale_code: rationale,
            tick_index,
        },
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

fn kernel_target(target: crate::protocol::CallTarget) -> Target {
    match target {
        crate::protocol::CallTarget::Empty => Target::Empty,
        crate::protocol::CallTarget::Place(id) => Target::Place(id),
        crate::protocol::CallTarget::Actor(id) => Target::Actor(id),
        crate::protocol::CallTarget::Clock(id) => Target::Clock(id),
    }
}
