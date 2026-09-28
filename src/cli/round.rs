use serde_json::{Map, Value};
use spookery_kernel::check::{CheckNotice, CheckOut};
use spookery_kernel::model::{ActorKind, ActorLive, Intent, IntentArgs, Source, Target};
use spookery_kernel::store::Shard;
use spookery_kernel::Code;
use spookery_tick::commit_turn_at;

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

pub fn gm_tool(shard: &mut Shard, name: &str, arguments: Option<Map<String, Value>>) -> ToolDone {
    match name {
        "intent" => match queue_tool_intent(shard, arguments) {
            Ok(note) => ToolDone {
                value: serde_json::json!({ "queued": note.id }),
                advanced: false,
                ticked: false,
                gm_event: None,
            },
            Err(code) => refused_tool(code),
        },
        "commit_turn" => match commit_tool(shard, arguments) {
            Ok(done) => done,
            Err(code) => refused_tool(code),
        },
        _ => refused_tool(Code::NotInSpec),
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
    Ok(ToolDone {
        value: serde_json::json!({
            "state_rev": outcome.state_rev,
            "diff_summary": outcome.diff_summary,
            "log_path": outcome.log_path,
        }),
        advanced,
        ticked: true,
        gm_event: None,
    })
}

fn noop(shard: &Shard) -> ToolDone {
    ToolDone {
        value: serde_json::json!({
            "state_rev": shard.now().state_rev,
            "diff_summary": [],
            "log_path": null,
        }),
        advanced: false,
        ticked: false,
        gm_event: None,
    }
}

fn refused_tool(code: Code) -> ToolDone {
    ToolDone {
        value: serde_json::json!({ "code": code.token() }),
        advanced: false,
        ticked: false,
        gm_event: None,
    }
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
