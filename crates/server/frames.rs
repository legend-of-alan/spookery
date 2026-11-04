//! Engine output as message bodies. The one place a `protocol::Refusal` is rendered: into a
//! body or an MCP error.

use crate::protocol::{
    self, CommitBody, Detail, NoteView, PackSummary, QueuedMove, Roll, Say, ServerBody,
    StatusPause, StatusPresence, TableStatus,
};
use rmcp::ErrorData as McpError;
use serde_json::Value;
use spookery_engine::{
    self as engine, Intent, MoveReport, Note, Presence, RoundOutput, State, TapeRecord,
};

/// A move the gate or a round refused. Its `detail` is the meter or item id, or the
/// sentence a locked exit shows.
pub(super) fn render_refused_move(refusal: &engine::Refusal) -> protocol::Refusal {
    protocol::Refusal {
        code: protocol::Code::Known(refusal.code),
        detail: refusal.detail.clone().map(Detail::One),
    }
}

/// The MCP initialize error. Its message is the refusal's sentence and its data the refusal.
pub(super) fn mcp_error(refusal: &protocol::Refusal) -> McpError {
    let data = serde_json::to_value(refusal).expect("refusal");
    McpError::internal_error(refusal.to_string(), Some(data))
}

/// One say as a seat hears it. Only the GM's copy lists the recipients.
pub(super) fn render_say(tape: &TapeRecord, gm: bool) -> ServerBody {
    ServerBody::Say(build_say(tape, gm))
}

fn build_say(tape: &TapeRecord, gm: bool) -> Say {
    Say {
        from: tape.from.clone(),
        address: tape.address.clone(),
        place: tape.place.clone(),
        actor: tape.actor.clone(),
        recipients: if gm {
            tape.recipients.clone()
        } else {
            Vec::new()
        },
        text: tape.text.clone(),
    }
}

/// The reply to a claim: the PC, and the says it heard before.
pub(super) fn render_claimed(actor: &str, scrollback: &[TapeRecord]) -> ServerBody {
    ServerBody::Claimed {
        actor: actor.to_string(),
        scrollback: scrollback
            .iter()
            .map(|tape| build_say(tape, false))
            .collect(),
    }
}

/// The `check` and `refused` bodies of this actor's moves.
pub(super) fn render_checks(reports: &[MoveReport], actor_id: &str) -> Vec<ServerBody> {
    reports
        .iter()
        .filter_map(|report| match report {
            MoveReport::Rolled { actor_id: id, roll } if id == actor_id => {
                Some(ServerBody::Check {
                    outcome: roll.outcome,
                })
            }
            MoveReport::Refused {
                actor_id: id,
                refusal,
            } if id == actor_id => Some(ServerBody::Refused(render_refused_move(refusal))),
            _ => None,
        })
        .collect()
}

fn rolls<'a>(reports: impl IntoIterator<Item = &'a MoveReport>) -> impl Iterator<Item = &'a Roll> {
    reports.into_iter().filter_map(|report| match report {
        MoveReport::Rolled { roll: out, .. } => Some(out),
        MoveReport::Refused { .. } => None,
    })
}

pub(super) fn render_resume(view: &engine::View) -> ServerBody {
    let actor_id = view.viewer.actor_id();
    let notes = view
        .notes
        .iter()
        .filter(|note| note.to.iter().any(|to| to == actor_id))
        .map(note_view)
        .collect();
    ServerBody::Resume {
        notes,
        clocks: view.clocks.clone(),
    }
}

pub(super) fn render_view(view: &engine::View) -> ServerBody {
    ServerBody::View(Box::new(protocol::View {
        viewer: view.viewer.actor_id().to_string(),
        place: view.place.clone(),
        place_name: view.place_name.clone(),
        body: view.prose.clone(),
        actor_names: view.actor_names.clone(),
        status: view.status.clone(),
        visible_actors: view.visible_actors.clone(),
        known_facts: view.known_facts.clone(),
        notes: view.notes.iter().map(note_view).collect(),
        unknown_mentions: view.unknown_mentions.clone(),
        clocks: view.clocks.clone(),
        items: view.items.clone(),
        meters: view.meters.clone(),
        habits: view.habits.clone(),
        exits: view.exits.clone(),
    }))
}

pub(super) fn render_committed(
    state_rev: u64,
    changed_keys: &[String],
    journal_path: Option<&str>,
    outputs: &[RoundOutput],
) -> ServerBody {
    let checks = rolls(outputs.iter().flat_map(|output| &output.reports))
        .cloned()
        .collect();
    let dice = outputs
        .iter()
        .flat_map(|output| output.dice.iter().cloned())
        .collect();
    ServerBody::Committed(CommitBody {
        state_rev,
        changed_keys: changed_keys.to_vec(),
        journal_path: journal_path.map(str::to_string),
        rolls: checks,
        dice,
    })
}

pub(super) fn render_status(
    now: &State,
    presence: &Presence,
    queued: &[Intent],
    pack: PackSummary,
) -> ServerBody {
    let Ok(Value::String(presence_clocks)) = serde_json::to_value(now.pause.presence_clocks) else {
        unreachable!("a run state serializes as its snake_case name");
    };
    ServerBody::Status(Box::new(TableStatus {
        presence: StatusPresence {
            players: presence.players,
            actors_present: presence.actors_present.clone(),
        },
        pause: StatusPause { presence_clocks },
        world_time: now.world_time,
        round_index: now.round_index,
        state_rev: now.state_rev,
        pack_rev: now.pack_rev.clone(),
        last_round_at: now.last_round_at.clone(),
        queued: queued
            .iter()
            .map(|intent| QueuedMove {
                id: intent.id.clone(),
                actor_id: intent.actor.actor_id().to_string(),
                move_name: intent.move_name.clone(),
            })
            .collect(),
        pack: Some(pack),
    }))
}

fn note_view(note: &Note) -> NoteView {
    NoteView {
        id: note.id.clone(),
        to: note.to.clone(),
        place: note.place.clone(),
        fact_ids: note.fact_ids.clone(),
        text: note.text.clone(),
    }
}
