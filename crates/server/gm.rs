//! The GM words: how each one runs, and how its MCP tool describes it.

use std::sync::Arc;

use rmcp::handler::server::tool::schema_for_type;
use serde_json::{Map, Value};
use spookery_engine::{Address, Presence, Reason, RoundOutput, TapeRecord, World};

use super::frames;
use super::table::commit_then_fast_forward;
use crate::protocol::{
    parse_args, CommitArgs, IntentArgs, NoArgs, PerceiveArgs, SayArgs, ServerBody,
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
        "queue" => queue(world, presence, args),
        "say" => say(world, presence, args),
        "commit" => commit(world, presence, args),
        "release" => release(args),
        _ => Err(Reason::Schema),
    }
}

/// What a GM word's MCP tool says about it, and the schema of the word's `args`. `None` for a
/// word that is not a GM word.
pub(super) fn describe_gm_word(word: &str) -> Option<(&'static str, Arc<Map<String, Value>>)> {
    let described = match word {
        "status" => (
            "status takes no keys. The reply is who is seated, the queue and the pack.",
            schema_for_type::<NoArgs>(),
        ),
        "perceive" => (
            "perceive requires as. max_body_chars may be sent. A missing as is UNSCOPED_PERCEIVE.",
            schema_for_type::<PerceiveArgs>(),
        ),
        "queue" => (
            "queue requires actor_id and move. A habit row is legal when actor_id is an npc. A PC a player holds is ACTOR_MISMATCH.",
            schema_for_type::<IntentArgs>(),
        ),
        "say" => (
            "say requires address and text. place is required when address is place. actor is required when address is actor. table takes neither place nor actor. A supplied from is SCHEMA.",
            schema_for_type::<SayArgs>(),
        ),
        "commit" => (
            "commit requires include_round. session_id may be a string. The round messages come before the reply.",
            schema_for_type::<CommitArgs>(),
        ),
        "release" => (
            "release takes no keys. The reply is sent after this GM seat is free.",
            schema_for_type::<NoArgs>(),
        ),
        _ => return None,
    };
    Some(described)
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
    parse_args::<NoArgs>(arguments).map_err(|_| Reason::Schema)?;
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
    parse_args::<NoArgs>(arguments).map_err(|_| Reason::Schema)?;
    Ok(Reply::new(frames::render_status(
        world.state(),
        presence,
        world.queued_intents(),
        world.pack_summary(),
    )))
}

/// A PC a player holds takes only that player's moves, so the GM's is `ACTOR_MISMATCH`.
fn queue(
    world: &mut World,
    presence: &Presence,
    arguments: Option<&Map<String, Value>>,
) -> Result<Reply, Reason> {
    let call: IntentArgs = parse_args(arguments).map_err(|_| Reason::Schema)?;
    if let Some(actor_id) = &call.actor_id {
        if presence.actors_present.contains(actor_id) {
            return Err(Reason::ActorMismatch);
        }
    }
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

    use spookery_engine::{Presence, Reason};

    use super::{describe_gm_word, run_gm_word};
    use crate::protocol::{QueuedMove, Seat, WORDS};

    /// `run_gm_word` runs each GM word the hello lists: well-formed arguments are not
    /// `SCHEMA`, and a word that is not one is.
    #[test]
    fn each_gm_word_runs() {
        let pack = copy_pack("oneshot-docks", "gm_words");
        let mut world = spookery_engine::testing::resume(&pack);
        let calls = [
            ("status", json!({})),
            ("perceive", json!({"as": "pc_rowan"})),
            ("queue", json!({"actor_id": "pc_rowan", "move": "wait"})),
            ("say", json!({"address": "table", "text": "hi"})),
            ("commit", json!({"include_round": false})),
            ("release", json!({})),
        ];
        let gm_words: Vec<&str> = WORDS
            .iter()
            .filter(|word| word.seat == Seat::Gm)
            .map(|word| word.name)
            .collect();
        let called: Vec<&str> = calls.iter().map(|(word, _)| *word).collect();
        assert_eq!(called, gm_words);
        for (word, args) in calls {
            let args = args.as_object().cloned().unwrap_or_default();
            let ran = run_gm_word(&mut world, &Presence::default(), word, &args);
            assert!(!matches!(ran, Err(Reason::Schema)), "{word}");
        }
        assert!(matches!(
            run_gm_word(&mut world, &Presence::default(), "fly", &Default::default()),
            Err(Reason::Schema)
        ));
    }

    /// Every GM word the hello lists has a tool description and schema. A word that is not
    /// a GM word has none.
    #[test]
    fn each_gm_word_is_described() {
        for word in WORDS.iter().filter(|word| word.seat == Seat::Gm) {
            assert!(describe_gm_word(word.name).is_some(), "{}", word.name);
        }
        assert!(describe_gm_word("fly").is_none());
    }

    #[test]
    fn parsed_intent_reaches_the_world_with_move_and_to() {
        let pack = copy_pack("oneshot-docks", "gm_intent");
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
