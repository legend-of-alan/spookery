use serde_json::{Map, Value};
use spookery_kernel::check::{CheckNotice, CheckOut};
use spookery_kernel::model::{ActorKind, ActorLive, Intent, IntentArgs, RunState, Source, Target};
use spookery_kernel::perceive::perceive_budget;
use spookery_kernel::store::Shard;
use spookery_kernel::Code;
use spookery_tick::commit_turn_at;

use super::speech;
use crate::schedule::{self, seconds_since, wall_now};

pub struct QueuedNote {
    pub id: String,
    pub actor_id: String,
    pub move_name: String,
}

pub enum Mechanical {
    Queued(QueuedNote),
    Committed {
        advanced: bool,
        notices: Vec<CheckNotice>,
    },
}

pub struct ToolDone {
    pub value: Value,
    pub advanced: bool,
    pub ticked: bool,
    pub gm_event: Option<Vec<u8>>,
    /// Written on the GM socket after this call's response line.
    pub after: Vec<Vec<u8>>,
    pub player_line: Option<(Vec<String>, Vec<u8>)>,
    pub tape: Option<String>,
}

impl ToolDone {
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

pub fn mechanical(shard: &mut Shard, actor_id: &str, line: &str, gm_seated: bool) -> Result<Mechanical, Code> {
    let (verb, rest) = verb_line(line, actor_id)?;
    match verb {
        "attempt" => {
            let note = queue_attempt(shard, actor_id, rest)?;
            if gm_seated {
                return Ok(Mechanical::Queued(note));
            }
            let advanced = commit_queued(shard)?;
            Ok(Mechanical::Committed {
                advanced,
                notices: shard.take_rolls(),
            })
        }
        "wait" if rest.is_empty() => {
            let note = queue_wait(shard, actor_id)?;
            if gm_seated {
                return Ok(Mechanical::Queued(note));
            }
            let advanced = commit_queued(shard)?;
            Ok(Mechanical::Committed {
                advanced,
                notices: shard.take_rolls(),
            })
        }
        "wait" => Err(Code::Schema),
        _ => Err(Code::NotInSpec),
    }
}

/// Sitting `tools/list` order. Each name is one handler or one refusal.
/// Any other name is `NOT_IN_SPEC`.
pub(super) struct GmTool {
    pub(super) name: &'static str,
    pub(super) about: &'static str,
    pub(super) schema: fn() -> Map<String, Value>,
    run: fn(&mut Shard, Option<Map<String, Value>>) -> ToolDone,
}

const SEAT_FACTS: &str = "Player mechanical lines stay queued until commit_turn. say appends speech and does not tick. perceive requires as set to player:<id> or npc:<id>. This seat does not call session_open or patch_entity. When a notification does not arrive, session_status with since returns player speech and the queue.";

impl GmTool {
    pub(super) fn description(&self) -> String {
        format!("{} {}", self.about, SEAT_FACTS)
    }
}

pub(super) const GM_LIST: &[GmTool] = &[
    GmTool {
        name: "session_open",
        about: "Calling session_open on this seat returns NOT_IN_SPEC and writes nothing.",
        schema: empty_schema,
        run: refuse_not_in_spec,
    },
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
        about: "intent requires actor_id, actor_kind, source, and move. source stays player.",
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
        name: "resolve_check",
        about: "Calling resolve_check on this seat returns NOT_IN_SPEC.",
        schema: empty_schema,
        run: refuse_not_in_spec,
    },
    GmTool {
        name: "apply_effect",
        about: "Calling apply_effect on this seat returns NOT_IN_SPEC.",
        schema: empty_schema,
        run: refuse_not_in_spec,
    },
    GmTool {
        name: "npc_act",
        about: "npc_act returns LLM_DISABLED and calls no model.",
        schema: empty_schema,
        run: refuse_llm_disabled,
    },
    GmTool {
        name: "commit_turn",
        about: "commit_turn requires include_tick. session_id may be a string.",
        schema: commit_schema,
        run: run_commit_turn,
    },
    GmTool {
        name: "recall",
        about: "Calling recall on this seat returns NOT_IN_SPEC.",
        schema: empty_schema,
        run: refuse_not_in_spec,
    },
    GmTool {
        name: "session_save",
        about: "Calling session_save on this seat returns NOT_IN_SPEC.",
        schema: empty_schema,
        run: refuse_not_in_spec,
    },
];

pub fn gm_tool(shard: &mut Shard, name: &str, arguments: Option<Map<String, Value>>) -> ToolDone {
    match GM_LIST.iter().find(|tool| tool.name == name) {
        Some(tool) => (tool.run)(shard, arguments),
        None => refuse_not_in_spec(shard, arguments),
    }
}

fn run_intent(shard: &mut Shard, arguments: Option<Map<String, Value>>) -> ToolDone {
    match queue_tool_intent(shard, arguments) {
        Ok(note) => ToolDone::new(serde_json::json!({ "queued": note.id })),
        Err(code) => refused_tool(code),
    }
}

fn run_commit_turn(shard: &mut Shard, arguments: Option<Map<String, Value>>) -> ToolDone {
    match commit_tool(shard, arguments) {
        Ok(done) => done,
        Err(code) => refused_tool(code),
    }
}

fn run_perceive(shard: &mut Shard, arguments: Option<Map<String, Value>>) -> ToolDone {
    match perceive_tool(shard, arguments) {
        Ok(value) => ToolDone::new(value),
        Err(code) => refused_tool(code),
    }
}

fn run_say(shard: &mut Shard, arguments: Option<Map<String, Value>>) -> ToolDone {
    match speech::speak_tool(shard, arguments) {
        Ok(spoken) => {
            let value = serde_json::from_str(&spoken.gm_object).unwrap_or(Value::Null);
            let mut done = ToolDone::new(value);
            // Same object as the tool result, after that response. Ahead of it, the client stalls.
            done.after.push(spoken.gm_event);
            done.player_line = Some((spoken.recipients, spoken.player_event.into_bytes()));
            done.tape = Some(spoken.tape);
            done
        }
        Err(code) => refused_tool(code),
    }
}

fn run_session_status(shard: &mut Shard, arguments: Option<Map<String, Value>>) -> ToolDone {
    match session_status_tool(shard, arguments) {
        Ok(value) => ToolDone::new(value),
        Err(code) => refused_tool(code),
    }
}

fn refuse_not_in_spec(_shard: &mut Shard, _arguments: Option<Map<String, Value>>) -> ToolDone {
    refused_tool(Code::NotInSpec)
}

fn refuse_llm_disabled(_shard: &mut Shard, _arguments: Option<Map<String, Value>>) -> ToolDone {
    refused_tool(Code::LlmDisabled)
}

fn empty_schema() -> Map<String, Value> {
    schema_object(serde_json::json!({ "type": "object" }))
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
            "rationale_code": { "type": "string" },
            "llm_say": { "type": ["string", "null"] }
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

/// Seat check and interval check. A tick that runs borrows this `Shard`.
/// Returns whether `world_time` advanced.
pub fn on_scheduled_wake(shard: &mut Shard, player_held: bool, gm_held: bool) -> Result<bool, Code> {
    if player_held || gm_held {
        return Ok(false);
    }
    let every = shard.budget().tick_every_real_seconds;
    if seconds_since(&shard.now().last_tick_at)? < every {
        return Ok(false);
    }
    let before = shard.now().world_time;
    schedule::wake_shard(shard)?;
    Ok(shard.now().world_time != before)
}

pub fn queue_attempt(shard: &mut Shard, actor_id: &str, rest: &str) -> Result<QueuedNote, Code> {
    if shard.now().presence.humans == 0 {
        return Err(Code::LiveOnly);
    }
    if !shard.now().actors.get(actor_id).is_some_and(ActorLive::is_pc) {
        return Err(Code::Schema);
    }
    let (check, rest) = split_token(rest);
    if check.is_empty() || rest.is_empty() {
        return Err(Code::Schema);
    }
    let (target_token, extra) = split_token(rest);
    if target_token.is_empty() {
        return Err(Code::Schema);
    }
    if !extra.is_empty() {
        return Err(Code::NotInSpec);
    }
    let Some(target) = integer_token(target_token) else {
        return Err(Code::NotInSpec);
    };
    let note = QueuedNote {
        id: mint_intent_id()?,
        actor_id: actor_id.to_string(),
        move_name: "attempt".to_string(),
    };
    push_intent(
        shard,
        note.id.clone(),
        actor_id,
        ActorKind::Player,
        "attempt",
        Target::Empty,
        IntentArgs {
            check: Some(check.to_string()),
            check_target: Some(target),
            ..IntentArgs::default()
        },
        String::new(),
        None,
    )?;
    Ok(note)
}

pub fn player_notice_lines(notices: &[CheckNotice], actor_id: &str) -> String {
    let mut out = String::new();
    for notice in notices {
        if notice.actor_id() != actor_id {
            continue;
        }
        match notice {
            CheckNotice::Rolled { out: rolled, .. } => {
                out.push_str("{\"kind\":\"check\",\"success\":");
                out.push_str(if rolled.success { "true" } else { "false" });
                out.push_str("}\n");
            }
            CheckNotice::Refused { code, .. } => {
                out.push_str("{\"kind\":\"refused\",\"code\":\"");
                out.push_str(code.token());
                out.push_str("\"}\n");
            }
        }
    }
    out
}

pub fn gm_check_frames(notices: &[CheckNotice]) -> Vec<Vec<u8>> {
    notices
        .iter()
        .filter_map(|notice| match notice {
            CheckNotice::Rolled { out, .. } => Some(gm_check_frame(out)),
            CheckNotice::Refused { .. } => None,
        })
        .collect()
}

fn gm_check_frame(out: &CheckOut) -> Vec<u8> {
    let mut faces = String::from("[");
    for (index, face) in out.faces.iter().enumerate() {
        if index > 0 {
            faces.push(',');
        }
        faces.push_str(&face.to_string());
    }
    faces.push(']');
    format!(
        "{{\"jsonrpc\":\"2.0\",\"method\":\"spook/event\",\"params\":{{\"kind\":\"check\",\"faces\":{faces},\"stat\":{},\"total\":{},\"target\":{},\"success\":{}}}}}\n",
        out.stat,
        out.total,
        out.target,
        if out.success { "true" } else { "false" }
    )
    .into_bytes()
}

fn integer_token(token: &str) -> Option<i64> {
    let digits = token.strip_prefix('-').unwrap_or(token);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    token.parse().ok()
}

pub fn queue_wait(shard: &mut Shard, actor_id: &str) -> Result<QueuedNote, Code> {
    if !shard.now().actors.get(actor_id).is_some_and(ActorLive::is_pc) {
        return Err(Code::Schema);
    }
    let note = QueuedNote {
        id: mint_intent_id()?,
        actor_id: actor_id.to_string(),
        move_name: "wait".to_string(),
    };
    push_intent(
        shard,
        note.id.clone(),
        actor_id,
        ActorKind::Player,
        "wait",
        Target::Empty,
        IntentArgs::default(),
        String::new(),
        None,
    )?;
    Ok(note)
}

/// An empty round does not enter `run_tick` and does not advance `world_time`.
pub fn commit_queued(shard: &mut Shard) -> Result<bool, Code> {
    if shard.queued_intents().is_empty() {
        return Ok(false);
    }
    let before = shard.now().world_time;
    let wall = wall_now()?;
    commit_turn_at(shard, &wall)?;
    Ok(shard.now().world_time != before)
}

impl QueuedNote {
    pub fn player_line(&self) -> String {
        format!(
            "{{\"kind\":\"queued\",\"id\":{}}}\n",
            serde_json::to_string(&self.id).unwrap_or_else(|_| "\"\"".to_string())
        )
    }

    pub fn gm_frame(&self) -> Vec<u8> {
        let id = serde_json::to_string(&self.id).unwrap_or_else(|_| "\"\"".to_string());
        let actor = serde_json::to_string(&self.actor_id).unwrap_or_else(|_| "\"\"".to_string());
        let move_name = serde_json::to_string(&self.move_name).unwrap_or_else(|_| "\"\"".to_string());
        format!(
            "{{\"jsonrpc\":\"2.0\",\"method\":\"spook/event\",\"params\":{{\"kind\":\"queued\",\"id\":{id},\"actor_id\":{actor},\"move\":{move_name}}}}}\n"
        )
        .into_bytes()
    }
}

fn commit_tool(shard: &mut Shard, arguments: Option<Map<String, Value>>) -> Result<ToolDone, Code> {
    let include_tick = commit_include_tick(arguments)?;
    if !include_tick {
        if shard.queued_intents().is_empty() {
            return Ok(noop(shard));
        }
        return Err(Code::NotInSpec);
    }
    if shard.queued_intents().is_empty() {
        return Ok(noop(shard));
    }
    let before = shard.now().world_time;
    let wall = wall_now()?;
    let outcome = commit_turn_at(shard, &wall)?;
    let advanced = shard.now().world_time != before;
    let mut done = ToolDone::new(serde_json::json!({
        "state_rev": outcome.state_rev,
        "diff_summary": outcome.diff_summary,
        "log_path": outcome.log_path,
    }));
    done.advanced = advanced;
    done.ticked = true;
    Ok(done)
}

fn noop(shard: &Shard) -> ToolDone {
    ToolDone::new(serde_json::json!({
        "state_rev": shard.now().state_rev,
        "diff_summary": [],
        "log_path": null,
    }))
}

fn refused_tool(code: Code) -> ToolDone {
    ToolDone::new(serde_json::json!({ "code": code.token() }))
}

fn perceive_tool(shard: &Shard, arguments: Option<Map<String, Value>>) -> Result<Value, Code> {
    let arguments = arguments.unwrap_or_default();
    let mut as_arg = None;
    let mut budget = None;
    for (key, value) in &arguments {
        match key.as_str() {
            "as" => as_arg = Some(value.as_str().ok_or(Code::Schema)?.to_string()),
            "focus" => match value.as_str() {
                Some("place" | "actor") => {}
                _ => return Err(Code::Schema),
            },
            "budget_tokens" => budget = Some(value.as_u64().ok_or(Code::Schema)?),
            _ => return Err(Code::Schema),
        }
    }
    let as_arg = as_arg.ok_or(Code::UnscopedPerceive)?;
    let budget = usize::try_from(budget.unwrap_or(800)).map_err(|_| Code::Schema)?;
    let view = perceive_budget(shard, &as_arg, budget)?;
    let body = super::perceive_object(&view);
    serde_json::from_str(&body).map_err(|_| Code::Schema)
}

fn session_status_tool(shard: &Shard, arguments: Option<Map<String, Value>>) -> Result<Value, Code> {
    let since = status_since(arguments)?;
    let body = status_json(shard, since)?;
    serde_json::from_str(&body).map_err(|_| Code::Schema)
}

fn status_since(arguments: Option<Map<String, Value>>) -> Result<Option<u64>, Code> {
    let Some(arguments) = arguments else {
        return Ok(None);
    };
    if arguments.is_empty() {
        return Ok(None);
    }
    if arguments.len() != 1 {
        return Err(Code::Schema);
    }
    let since = arguments.get("since").ok_or(Code::Schema)?;
    Ok(Some(since.as_u64().ok_or(Code::Schema)?))
}

fn status_json(shard: &Shard, since: Option<u64>) -> Result<String, Code> {
    let now = shard.now();
    let mut out = String::from("{\"presence\":{\"humans\":");
    out.push_str(&now.presence.humans.to_string());
    out.push_str(",\"session_ids\":");
    out.push_str(&serde_json::to_string(&now.presence.session_ids).unwrap_or_else(|_| "[]".to_string()));
    out.push_str(",\"actors_present\":");
    out.push_str(&serde_json::to_string(&now.presence.actors_present).unwrap_or_else(|_| "[]".to_string()));
    out.push_str("},\"pause\":{\"named_plot_clocks\":");
    push_state(&mut out, now.pause.named_plot_clocks);
    out.push_str(",\"relationship_drift\":");
    push_state(&mut out, now.pause.relationship_drift);
    out.push_str(",\"subsistence_clocks\":");
    push_state(&mut out, now.pause.subsistence_clocks);
    out.push_str(",\"travel\":");
    push_state(&mut out, now.pause.travel);
    out.push_str("},\"world_time\":");
    out.push_str(&now.world_time.to_string());
    out.push_str(",\"tick_index\":");
    out.push_str(&now.tick_index.to_string());
    out.push_str(",\"state_rev\":");
    out.push_str(&now.state_rev.to_string());
    out.push_str(",\"pack_rev\":");
    out.push_str(&serde_json::to_string(&now.pack_rev).unwrap_or_else(|_| "\"\"".to_string()));
    out.push_str(",\"last_tick_at\":");
    out.push_str(&serde_json::to_string(&now.last_tick_at).unwrap_or_else(|_| "\"\"".to_string()));
    out.push_str(",\"queued\":[");
    for (index, intent) in shard.queued_intents().iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str("{\"id\":");
        out.push_str(&serde_json::to_string(&intent.id).unwrap_or_else(|_| "\"\"".to_string()));
        out.push_str(",\"actor_id\":");
        out.push_str(&serde_json::to_string(&intent.actor_id).unwrap_or_else(|_| "\"\"".to_string()));
        out.push_str(",\"move\":");
        out.push_str(&serde_json::to_string(&intent.move_name).unwrap_or_else(|_| "\"\"".to_string()));
        out.push('}');
    }
    out.push_str("],\"last_seq\":");
    out.push_str(&speech::last_tape_seq(shard.root())?.to_string());
    if let Some(since) = since {
        out.push_str(",\"events\":[");
        for (index, event) in speech::tape_events_after(shard.root(), since)?.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            out.push_str(event);
        }
        out.push(']');
    }
    out.push('}');
    Ok(out)
}

fn push_state(out: &mut String, state: RunState) {
    out.push('"');
    out.push_str(match state {
        RunState::Paused => "paused",
        RunState::Running => "running",
    });
    out.push('"');
}

fn commit_include_tick(arguments: Option<Map<String, Value>>) -> Result<bool, Code> {
    let arguments = arguments.ok_or(Code::Schema)?;
    let mut include = None;
    for (key, value) in &arguments {
        match key.as_str() {
            "include_tick" => {
                include = Some(value.as_bool().ok_or(Code::Schema)?);
            }
            "session_id" => {
                if !value.is_string() {
                    return Err(Code::Schema);
                }
            }
            _ => return Err(Code::Schema),
        }
    }
    include.ok_or(Code::Schema)
}

fn queue_tool_intent(shard: &mut Shard, arguments: Option<Map<String, Value>>) -> Result<QueuedNote, Code> {
    let arguments = arguments.ok_or(Code::Schema)?;
    let mut id = None;
    let mut tick_index = None;
    let mut actor_id = None;
    let mut actor_kind = None;
    let mut source = None;
    let mut move_name = None;
    let mut target = Target::Empty;
    let mut args = IntentArgs::default();
    let mut rationale = String::new();
    for (key, value) in &arguments {
        match key.as_str() {
            "id" => id = Some(value.as_str().ok_or(Code::Schema)?.to_string()),
            "tick_index" => tick_index = Some(value.as_u64().ok_or(Code::Schema)?),
            "actor_id" => actor_id = Some(value.as_str().ok_or(Code::Schema)?.to_string()),
            "actor_kind" => actor_kind = Some(parse_kind(value.as_str().ok_or(Code::Schema)?)?),
            "source" => source = Some(value.as_str().ok_or(Code::Schema)?.to_string()),
            "move" => move_name = Some(value.as_str().ok_or(Code::Schema)?.to_string()),
            "target" => target = parse_target(value)?,
            "args" => args = parse_args(value)?,
            "rationale_code" => rationale = value.as_str().ok_or(Code::Schema)?.to_string(),
            "llm_say" => {
                if !value.is_null() && !value.is_string() {
                    return Err(Code::Schema);
                }
            }
            _ => return Err(Code::Schema),
        }
    }
    if source.as_deref() != Some("player") {
        return Err(Code::Schema);
    }
    let actor_id = actor_id.ok_or(Code::Schema)?;
    let actor_kind = actor_kind.ok_or(Code::Schema)?;
    let move_name = move_name.ok_or(Code::Schema)?;
    if move_name == "attempt" && shard.now().presence.humans == 0 {
        return Err(Code::LiveOnly);
    }
    if !shard.canon().moves.contains_key(&move_name) {
        return Err(Code::UnknownMove);
    }
    match actor_kind {
        ActorKind::Player => {
            if !shard.now().actors.get(&actor_id).is_some_and(ActorLive::is_pc) {
                return Err(Code::Schema);
            }
        }
        ActorKind::Npc => {
            if !shard.now().actors.get(&actor_id).is_some_and(ActorLive::is_npc) {
                return Err(Code::Schema);
            }
        }
        ActorKind::System => return Err(Code::Schema),
    }
    let id = match id {
        Some(id) if !id.is_empty() => id,
        Some(_) => return Err(Code::Schema),
        None => mint_intent_id()?,
    };
    if shard.queued_intents().iter().any(|intent| intent.id == id) {
        return Err(Code::Schema);
    }
    push_intent(
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

fn push_intent(
    shard: &mut Shard,
    id: String,
    actor_id: &str,
    actor_kind: ActorKind,
    move_name: &str,
    target: Target,
    args: IntentArgs,
    rationale_code: String,
    tick_index: Option<u64>,
) -> Result<(), Code> {
    let tick_index = match tick_index {
        Some(tick_index) => tick_index,
        None => shard.now().tick_index.checked_add(1).ok_or(Code::Schema)?,
    };
    shard.queue_intent(Intent {
        id,
        tick_index,
        actor_id: actor_id.to_string(),
        actor_kind,
        source: Source::Player,
        move_name: move_name.to_string(),
        target,
        args,
        rationale_code,
    });
    Ok(())
}

fn verb_line<'a>(line: &'a str, seated: &str) -> Result<(&'a str, &'a str), Code> {
    if line.is_empty() {
        return Err(Code::Schema);
    }
    let (first, rest) = split_token(line);
    if let Some(id) = first.strip_prefix("player:") {
        if id.is_empty() || id != seated || rest.is_empty() {
            return Err(Code::Schema);
        }
        return Ok(split_token(rest));
    }
    Ok((first, rest))
}

fn split_token(line: &str) -> (&str, &str) {
    match line.split_once(' ') {
        Some((token, rest)) => (token, rest),
        None => (line, ""),
    }
}

fn parse_kind(text: &str) -> Result<ActorKind, Code> {
    match text {
        "npc" => Ok(ActorKind::Npc),
        "player" => Ok(ActorKind::Player),
        "system" => Ok(ActorKind::System),
        _ => Err(Code::Schema),
    }
}

fn parse_target(value: &Value) -> Result<Target, Code> {
    let object = value.as_object().ok_or(Code::Schema)?;
    if object.is_empty() {
        return Ok(Target::Empty);
    }
    if object.len() != 1 {
        return Err(Code::Schema);
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
    if let Some(rumor) = object.get("rumor").and_then(Value::as_str) {
        return Ok(Target::Rumor(rumor.to_string()));
    }
    Err(Code::Schema)
}

fn parse_args(value: &Value) -> Result<IntentArgs, Code> {
    let object = value.as_object().ok_or(Code::Schema)?;
    let mut args = IntentArgs::default();
    for (key, item) in object {
        match key.as_str() {
            "sku" => args.sku = Some(item.as_str().ok_or(Code::Schema)?.to_string()),
            "delta" => args.delta = Some(item.as_i64().ok_or(Code::Schema)?),
            "key" => args.key = Some(item.as_str().ok_or(Code::Schema)?.to_string()),
            "to" => match item {
                Value::String(text) => args.to = Some(text.clone()),
                Value::Array(values) => {
                    let mut ids = Vec::new();
                    for value in values {
                        ids.push(value.as_str().ok_or(Code::Schema)?.to_string());
                    }
                    args.message_to = Some(ids);
                }
                _ => return Err(Code::Schema),
            },
            "message_id" => args.message_id = Some(item.as_str().ok_or(Code::Schema)?.to_string()),
            "due_clock" => args.due_clock = Some(item.as_str().ok_or(Code::Schema)?.to_string()),
            "due_value" => args.due_value = Some(item.as_i64().ok_or(Code::Schema)?),
            "place" => args.place = Some(item.as_str().ok_or(Code::Schema)?.to_string()),
            "fact_ids" => {
                let values = item.as_array().ok_or(Code::Schema)?;
                let mut ids = Vec::new();
                for value in values {
                    ids.push(value.as_str().ok_or(Code::Schema)?.to_string());
                }
                args.fact_ids = Some(ids);
            }
            "text" => args.text = Some(item.as_str().ok_or(Code::Schema)?.to_string()),
            "proposal_id" => args.proposal_id = Some(item.as_str().ok_or(Code::Schema)?.to_string()),
            "kind" => args.kind = Some(item.as_str().ok_or(Code::Schema)?.to_string()),
            "check" => args.check = Some(item.as_str().ok_or(Code::Schema)?.to_string()),
            "target" => match item {
                Value::String(text) => args.target = Some(text.clone()),
                Value::Number(_) => args.check_target = Some(item.as_i64().ok_or(Code::Schema)?),
                _ => return Err(Code::Schema),
            },
            "summary" => args.summary = Some(item.as_str().ok_or(Code::Schema)?.to_string()),
            _ => return Err(Code::Schema),
        }
    }
    Ok(args)
}

fn mint_intent_id() -> Result<String, Code> {
    let mut bytes = [0u8; 16];
    if !fill_random(&mut bytes) {
        return Err(Code::Schema);
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
