//! The GM commands: how each one runs.

use serde_json::{Map, Value};
use spookery_protocol::{
    parse_args, CommitArgs, NoArgs, PerceiveArgs, QueueArgs, SayArgs, ServerBody,
};

use super::message_body;
use crate::{ActorId, Presence, Reason, Session, TapeRecord};

/// Sent from a GM command's run back to the router, which answers the GM and updates seats.
pub(super) struct Reply {
    pub(super) body: ServerBody,
    /// The speech this command put on the tape. The router delivers it, and the GM's copy is
    /// the reply.
    pub(super) speech: Option<TapeRecord>,
    /// The GM seat is cleared once this reply is sent.
    pub(super) release_seat: bool,
    /// Rounds this command played, commit first.
    pub(super) outputs: Vec<crate::round::Output>,
}

impl Reply {
    fn new(body: ServerBody) -> Self {
        Self {
            body,
            speech: None,
            release_seat: false,
            outputs: Vec::new(),
        }
    }

    /// The reply to a refused command.
    pub(super) fn refused(code: Reason) -> Self {
        Self::new(ServerBody::Refused(code.into()))
    }
}

/// Run one GM command. `presence` is who the seats hold, and `wall` stamps any round it plays. A
/// command that is not a GM command is `SCHEMA`.
pub(super) fn run(
    session: &mut Session,
    presence: &Presence,
    command: &str,
    args: &Map<String, Value>,
    wall: &str,
) -> Result<Reply, Reason> {
    let args = Some(args);
    match command {
        "status" => status(session, presence, args),
        "perceive" => perceive(session, presence, args),
        "queue" => queue(session, presence, args),
        "say" => say(session, presence, args),
        "commit" => commit(session, presence, args, wall),
        "release" => release(args),
        _ => Err(Reason::Schema),
    }
}

/// GM `say`. `from` is the seat, or the NPC `as` names. A supplied `from` appends nothing.
fn say(
    session: &mut Session,
    presence: &Presence,
    arguments: Option<&Map<String, Value>>,
) -> Result<Reply, Reason> {
    let args: SayArgs = parse_args(arguments).map_err(|_| Reason::Schema)?;
    let npc_id = args.npc_id.as_deref().map(ActorId::from);
    let speech = session.say_as_gm(presence, npc_id.as_ref(), args.to.as_ref(), &args.text)?;
    let mut reply = Reply::new(ServerBody::Say(message_body::build_speech(&speech, true)));
    reply.speech = Some(speech);
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
    session: &mut Session,
    presence: &Presence,
    arguments: Option<&Map<String, Value>>,
    wall: &str,
) -> Result<Reply, Reason> {
    let include_round = parse_args::<CommitArgs>(arguments)
        .map_err(|_| Reason::Schema)?
        .include_round;
    let committed = !session.queued_intents().is_empty();
    if committed && !include_round {
        return Err(Reason::NotInSpec);
    }
    let outputs = if include_round {
        session.commit_then_fast_forward(presence, wall)?
    } else {
        Vec::new()
    };
    let (state_rev, changed_keys, journal_path) = match committed.then(|| &outputs[0]) {
        Some(output) => {
            let outcome = output
                .commit
                .as_ref()
                .expect("a flush round writes the journal");
            let journal_path = Some(outcome.journal_path.as_str());
            (
                outcome.state_rev,
                output.changed_keys.as_slice(),
                journal_path,
            )
        }
        None => (session.world().state_rev, &[][..], None),
    };
    let body = message_body::build_committed(state_rev, changed_keys, journal_path, &outputs);
    Ok(Reply {
        outputs,
        ..Reply::new(body)
    })
}

fn perceive(
    session: &mut Session,
    presence: &Presence,
    arguments: Option<&Map<String, Value>>,
) -> Result<Reply, Reason> {
    let args: PerceiveArgs = parse_args(arguments).map_err(|_| Reason::Schema)?;
    let viewer_id = args.viewer_id.ok_or(Reason::UnscopedPerceive)?;
    let view = session.perceive(presence, &viewer_id)?;
    Ok(Reply::new(message_body::build_view(&view)))
}

fn status(
    session: &mut Session,
    presence: &Presence,
    arguments: Option<&Map<String, Value>>,
) -> Result<Reply, Reason> {
    parse_args::<NoArgs>(arguments).map_err(|_| Reason::Schema)?;
    Ok(Reply::new(message_body::build_status(
        session.world(),
        presence,
        session.queued_intents(),
        session.pack_summary(),
    )))
}

/// A PC a player holds takes only that player's moves, so the GM's is `ACTOR_MISMATCH`.
fn queue(
    session: &mut Session,
    presence: &Presence,
    arguments: Option<&Map<String, Value>>,
) -> Result<Reply, Reason> {
    let call: QueueArgs = parse_args(arguments).map_err(|_| Reason::Schema)?;
    if let Some(actor_id) = &call.actor_id {
        if presence
            .actors_present
            .iter()
            .any(|id| id == actor_id.as_str())
        {
            return Err(Reason::ActorMismatch);
        }
    }
    let target = call.target.clone().filter(|target| !target.is_empty());
    let new_id = || Ok(uuid::Uuid::new_v4().to_string());
    let queued = session.queue_gm_intent(presence, call, new_id)?;
    Ok(Reply::new(ServerBody::Queued {
        intent: queued.id,
        actor_id: queued.actor_id,
        move_id: queued.move_id,
        target,
    }))
}
