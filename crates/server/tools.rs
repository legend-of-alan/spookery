//! The GM tool table: each tool's name, description, schema and run.

use std::sync::Arc;

use rmcp::handler::server::tool::schema_for_type;
use serde_json::{Map, Value};
use spookery_engine::{Address, Presence, Reason, RoundOutput, TapeRecord, World};

use super::frames;
use super::table::commit_then_fast_forward;
use crate::protocol::{
    parse_args, CloseArgs, CloseBody, CommitArgs, IntentCall, PerceiveArgs, Queued, SayArgs,
    StatusArgs, PERCEIVE_BODY_CHARS,
};

/// Sent from a GM tool handler back to the table, which answers the GM and updates seats.
pub(super) struct Reply {
    pub(super) value: Value,
    /// The speech this call put on the tape. The host delivers it.
    pub(super) said: Option<TapeRecord>,
    /// The GM seat is cleared before this reply is sent.
    pub(super) release_seat: bool,
    /// Set when the call was refused. The host logs it.
    pub(super) refused: Option<Reason>,
    /// Rounds this call played, commit first.
    pub(super) outputs: Vec<RoundOutput>,
}

impl Reply {
    fn new(value: Value) -> Self {
        Self {
            value,
            said: None,
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
    pub(super) schema: fn() -> Arc<Map<String, Value>>,
    run: ToolFn,
}

/// A refused call returns its code, and `call_gm_tool` renders it.
type ToolFn = fn(&mut World, &Presence, Option<Map<String, Value>>) -> Result<Reply, Reason>;

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
        schema: schema_for_type::<StatusArgs>,
        run: table_status,
    },
    Tool {
        name: "perceive",
        about: "perceive requires as. max_body_chars may be sent. A missing as is UNSCOPED_PERCEIVE.",
        schema: schema_for_type::<PerceiveArgs>,
        run: perceive,
    },
    Tool {
        name: "queue_intent",
        about: "queue_intent requires actor_id and move. A habit row is legal when actor_id is an npc.",
        schema: schema_for_type::<IntentCall>,
        run: queue_intent,
    },
    Tool {
        name: "say",
        about: "say requires address and text. place is required when address is place. actor is required when address is actor. table takes neither place nor actor. A supplied from is SCHEMA.",
        schema: schema_for_type::<SayArgs>,
        run: say,
    },
    Tool {
        name: "commit_round",
        about: "commit_round requires include_round. session_id may be a string.",
        schema: schema_for_type::<CommitArgs>,
        run: commit_round,
    },
    Tool {
        name: "close",
        about: "close takes no keys. The result is sent after this GM seat is free.",
        schema: schema_for_type::<CloseArgs>,
        run: close,
    },
];

/// Run one GM tool. `presence` is who the host's seats hold.
pub(super) fn call_gm_tool(
    world: &mut World,
    presence: &Presence,
    name: &str,
    arguments: Option<Map<String, Value>>,
) -> Reply {
    let reply = match TOOLS.iter().find(|tool| tool.name == name) {
        Some(tool) => (tool.run)(world, presence, arguments),
        None => Err(Reason::NotInSpec),
    };
    reply.unwrap_or_else(|code| Reply {
        refused: Some(code),
        ..Reply::new(frames::refused_value(code))
    })
}

/// GM `say`. `from` is the seat. A supplied `from` appends nothing.
/// The GM also hears it as a `say` event, the same object as this result.
fn say(
    world: &mut World,
    presence: &Presence,
    arguments: Option<Map<String, Value>>,
) -> Result<Reply, Reason> {
    let args: SayArgs = parse_args(arguments.as_ref()).map_err(|_| Reason::Schema)?;
    let heard = match &args {
        SayArgs::Table { .. } => Address::Table,
        SayArgs::Place { place, .. } => Address::Place(place),
        SayArgs::Actor { actor, .. } => Address::Actor(actor),
    };
    let said = world.say_as_gm(presence, heard, args.text())?;
    let mut reply = Reply::new(frames::say_value(&said));
    reply.said = Some(said);
    Ok(reply)
}

fn close(
    _world: &mut World,
    _presence: &Presence,
    arguments: Option<Map<String, Value>>,
) -> Result<Reply, Reason> {
    parse_args::<CloseArgs>(arguments.as_ref()).map_err(|_| Reason::Schema)?;
    Ok(Reply {
        release_seat: true,
        ..Reply::new(serde_json::to_value(CloseBody { released: true }).expect("close"))
    })
}

/// Without `include_round` only an empty queue commits, and it plays nothing.
fn commit_round(
    world: &mut World,
    presence: &Presence,
    arguments: Option<Map<String, Value>>,
) -> Result<Reply, Reason> {
    let include_round = parse_args::<CommitArgs>(arguments.as_ref())
        .map_err(|_| Reason::Schema)?
        .include_round;
    let committed = !world.queued_intents().is_empty();
    if committed && !include_round {
        return Err(Reason::NotInSpec);
    }
    let outputs = if include_round {
        commit_then_fast_forward(world, presence)?
    } else {
        Vec::new()
    };
    // The queued round is the first output. Fast-forward rounds name no keys.
    let (state_rev, changed_keys, journal_path) = match committed.then(|| &outputs[0]) {
        Some(output) => {
            let outcome = output
                .commit
                .as_ref()
                .expect("a flush round writes the journal");
            let journal_path = Some(outcome.log_path.as_str());
            (outcome.state_rev, output.summary.as_slice(), journal_path)
        }
        None => (world.state().state_rev, &[][..], None),
    };
    let value = frames::commit_value(state_rev, changed_keys, journal_path, &outputs);
    Ok(Reply {
        outputs,
        ..Reply::new(value)
    })
}

fn perceive(
    world: &mut World,
    presence: &Presence,
    arguments: Option<Map<String, Value>>,
) -> Result<Reply, Reason> {
    let args: PerceiveArgs = parse_args(arguments.as_ref()).map_err(|_| Reason::Schema)?;
    let as_arg = args.as_viewer.ok_or(Reason::UnscopedPerceive)?;
    let budget = args
        .max_body_chars
        .map_or(Ok(PERCEIVE_BODY_CHARS), usize::try_from)
        .map_err(|_| Reason::Schema)?;
    let view = world.perceive_trimmed(presence, &as_arg, budget)?;
    Ok(Reply::new(frames::perceive_value(&view)))
}

fn table_status(
    world: &mut World,
    presence: &Presence,
    arguments: Option<Map<String, Value>>,
) -> Result<Reply, Reason> {
    let since = parse_args::<StatusArgs>(arguments.as_ref())
        .map_err(|_| Reason::Schema)?
        .since;
    let events = match since {
        Some(since) => Some(world.tape_after(since)?),
        None => None,
    };
    let last_seq = world.last_tape_seq()?;
    Ok(Reply::new(frames::status_value(
        world.state(),
        presence,
        world.queued_intents(),
        last_seq,
        events.as_deref(),
        since.is_none().then(|| world.pack_summary()),
    )))
}

fn queue_intent(
    world: &mut World,
    _presence: &Presence,
    arguments: Option<Map<String, Value>>,
) -> Result<Reply, Reason> {
    let call = parse_args(arguments.as_ref()).map_err(|_| Reason::Schema)?;
    let queued = world.queue_gm_intent(call, || Ok(uuid::Uuid::new_v4().to_string()))?;
    Ok(Reply::new(
        serde_json::to_value(Queued { id: queued.id }).expect("queued"),
    ))
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use spookery_engine::{testing::copy_pack, QueuedMove, World};

    #[test]
    fn parsed_intent_reaches_the_world_with_move_and_to() {
        let pack = copy_pack("oneshot-docks", "tools_intent");
        let mut world = World::open(&pack).unwrap();
        let args = json!({"actor_id": "pc_rowan", "move": "wait", "to": ["pc_wren"]});
        let call = crate::protocol::parse_args(args.as_object()).unwrap();
        let queued = world.queue_gm_intent(call, || Ok("i1".into())).unwrap();
        let want = QueuedMove {
            id: "i1".into(),
            actor_id: "pc_rowan".into(),
            move_name: "wait".into(),
        };
        assert_eq!(queued, want);
        let intent = &world.queued_intents()[0];
        assert_eq!(intent.move_name, "wait");
        assert_eq!(intent.args.message_to, Some(vec!["pc_wren".to_string()]));
    }
}
