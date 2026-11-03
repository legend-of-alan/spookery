//! The GM words: each word's run, and the MCP tool each one is listed as.

use std::sync::Arc;

use rmcp::handler::server::tool::schema_for_type;
use serde_json::{Map, Value};
use spookery_engine::{Address, Presence, Reason, RoundOutput, TapeRecord, World};

use super::frames;
use super::table::commit_then_fast_forward;
use crate::protocol::{
    self, parse_args, CloseArgs, CommitArgs, PerceiveArgs, SayArgs, ServerBody, StatusArgs,
    PERCEIVE_BODY_CHARS,
};

/// Sent from a GM word's run back to the table, which answers the GM and updates seats.
pub(super) struct Reply {
    pub(super) body: ServerBody,
    /// The speech this word put on the tape. The table delivers it, and the GM's copy is
    /// the reply.
    pub(super) said: Option<TapeRecord>,
    /// The GM seat is cleared once this reply is sent.
    pub(super) release_seat: bool,
    /// Rounds this word played, commit first.
    pub(super) outputs: Vec<RoundOutput>,
}

impl Reply {
    fn new(body: ServerBody) -> Self {
        Self {
            body,
            said: None,
            release_seat: false,
            outputs: Vec::new(),
        }
    }

    /// The reply to a refused word.
    pub(super) fn refused(code: Reason) -> Self {
        Self::new(ServerBody::Refused(code.into()))
    }
}

/// One MCP tool, in `tools/list` order: the GM word it runs, its description and schema.
/// Any other name is `NOT_IN_SPEC` and writes nothing.
pub(super) struct Tool {
    pub(super) name: &'static str,
    pub(super) word: &'static str,
    pub(super) about: &'static str,
    pub(super) schema: fn() -> Arc<Map<String, Value>>,
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
        word: "status",
        about: "table_status takes no required key. An empty object stays legal. since is the only other accepted key.",
        schema: schema_for_type::<StatusArgs>,
    },
    Tool {
        name: "perceive",
        word: "perceive",
        about: "perceive requires as. max_body_chars may be sent. A missing as is UNSCOPED_PERCEIVE.",
        schema: schema_for_type::<PerceiveArgs>,
    },
    Tool {
        name: "queue_intent",
        word: "queue",
        about: "queue_intent requires actor_id and move. A habit row is legal when actor_id is an npc.",
        schema: schema_for_type::<protocol::IntentArgs>,
    },
    Tool {
        name: "say",
        word: "say",
        about: "say requires address and text. place is required when address is place. actor is required when address is actor. table takes neither place nor actor. A supplied from is SCHEMA.",
        schema: schema_for_type::<SayArgs>,
    },
    Tool {
        name: "commit_round",
        word: "commit",
        about: "commit_round requires include_round. session_id may be a string.",
        schema: schema_for_type::<CommitArgs>,
    },
    Tool {
        name: "close",
        word: "release",
        about: "close takes no keys. The result is sent after this GM seat is free.",
        schema: schema_for_type::<CloseArgs>,
    },
];

/// Run one GM word. `presence` is who the seats hold. A word that is not a GM word is
/// `SCHEMA`.
pub(super) fn run_gm_word(
    world: &mut World,
    presence: &Presence,
    word: &str,
    args: &Map<String, Value>,
) -> Result<Reply, Reason> {
    let args = Some(args);
    match word {
        "status" => status(world, presence, args),
        "perceive" => perceive(world, presence, args),
        "queue" => queue(world, args),
        "say" => say(world, presence, args),
        "commit" => commit(world, presence, args),
        "release" => release(args),
        _ => Err(Reason::Schema),
    }
}

/// GM `say`. `from` is the seat. A supplied `from` appends nothing.
fn say(
    world: &mut World,
    presence: &Presence,
    arguments: Option<&Map<String, Value>>,
) -> Result<Reply, Reason> {
    let args: SayArgs = parse_args(arguments).map_err(|_| Reason::Schema)?;
    let heard = match &args {
        SayArgs::Table { .. } => Address::Table,
        SayArgs::Place { place, .. } => Address::Place(place),
        SayArgs::Actor { actor, .. } => Address::Actor(actor),
    };
    let said = world.say_as_gm(presence, heard, args.text())?;
    let mut reply = Reply::new(frames::render_say(&said, true));
    reply.said = Some(said);
    Ok(reply)
}

fn release(arguments: Option<&Map<String, Value>>) -> Result<Reply, Reason> {
    parse_args::<CloseArgs>(arguments).map_err(|_| Reason::Schema)?;
    Ok(Reply {
        release_seat: true,
        ..Reply::new(ServerBody::Released)
    })
}

/// Without `include_round` only an empty queue commits, and it plays nothing.
fn commit(
    world: &mut World,
    presence: &Presence,
    arguments: Option<&Map<String, Value>>,
) -> Result<Reply, Reason> {
    let include_round = parse_args::<CommitArgs>(arguments)
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
            (
                outcome.state_rev,
                output.changed_keys.as_slice(),
                journal_path,
            )
        }
        None => (world.state().state_rev, &[][..], None),
    };
    let body = frames::render_committed(state_rev, changed_keys, journal_path, &outputs);
    Ok(Reply {
        outputs,
        ..Reply::new(body)
    })
}

fn perceive(
    world: &mut World,
    presence: &Presence,
    arguments: Option<&Map<String, Value>>,
) -> Result<Reply, Reason> {
    let args: PerceiveArgs = parse_args(arguments).map_err(|_| Reason::Schema)?;
    let as_arg = args.as_viewer.ok_or(Reason::UnscopedPerceive)?;
    let budget = args
        .max_body_chars
        .map_or(Ok(PERCEIVE_BODY_CHARS), usize::try_from)
        .map_err(|_| Reason::Schema)?;
    let view = world.perceive_trimmed(presence, &as_arg, budget)?;
    Ok(Reply::new(frames::render_view(&view)))
}

fn status(
    world: &mut World,
    presence: &Presence,
    arguments: Option<&Map<String, Value>>,
) -> Result<Reply, Reason> {
    let since = parse_args::<StatusArgs>(arguments)
        .map_err(|_| Reason::Schema)?
        .since;
    let events = match since {
        Some(since) => Some(world.tape_after(since)?),
        None => None,
    };
    let last_seq = world.last_tape_seq()?;
    Ok(Reply::new(frames::render_status(
        world.state(),
        presence,
        world.queued_intents(),
        last_seq,
        events.as_deref(),
        since.is_none().then(|| world.pack_summary()),
    )))
}

fn queue(world: &mut World, arguments: Option<&Map<String, Value>>) -> Result<Reply, Reason> {
    let call: protocol::IntentArgs = parse_args(arguments).map_err(|_| Reason::Schema)?;
    let target = call.target.clone().filter(|target| !target.is_empty());
    let queued = world.queue_gm_intent(call, || Ok(uuid::Uuid::new_v4().to_string()))?;
    Ok(Reply::new(ServerBody::Queued {
        intent: queued.id,
        actor: queued.actor_id,
        move_name: queued.move_name,
        target,
    }))
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use spookery_engine::{testing::copy_pack, World};

    use super::TOOLS;
    use crate::protocol::{QueuedMove, Seat, WORDS};

    /// Each MCP tool runs a GM word the hello lists, and each GM word has one tool.
    #[test]
    fn the_tools_are_the_gm_words() {
        let mut tools: Vec<&str> = TOOLS.iter().map(|tool| tool.word).collect();
        let mut words: Vec<&str> = WORDS
            .iter()
            .filter(|word| word.seat == Seat::Gm)
            .map(|word| word.name)
            .collect();
        tools.sort_unstable();
        words.sort_unstable();
        assert_eq!(tools, words);
    }

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
