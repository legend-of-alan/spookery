//! Player lines, `spook/event` frames, and tool results. The one place a
//! `protocol::Refusal` is rendered: into a player line, a tool result or an MCP error.

use crate::protocol::{
    self, CommitBody, Detail, GmEvent, NoteView, Outcome, PackSummary, PlayerFrame, Queued,
    QueuedMove, Refusal, SayEvent, StatusPause, StatusPresence, TableStatus,
};
use rmcp::ErrorData as McpError;
use serde_json::Value;
use spookery_engine::{
    self as engine, GateRefusal, Intent, MoveReport, Note, Presence, Reason, Roll, RoundOutput,
    State, TapeRecord,
};

/// The player `refused` line. `detail` is left out when there is none.
pub(super) fn refused_line(refusal: Refusal) -> String {
    PlayerFrame::Refused(refusal).to_line()
}

/// The `refused` line of a move the gate refused. Its `missing` is the meter or item id,
/// or the sentence a locked exit shows.
pub(super) fn refused_move_line(refusal: &GateRefusal) -> String {
    refused_line(Refusal {
        code: protocol::Code::Known(refusal.reason()),
        detail: refusal.missing().map(|text| Detail::One(text.to_string())),
    })
}

/// A refused GM tool's result names the code only.
pub(super) fn refused_value(code: Reason) -> Value {
    serde_json::to_value(Refusal::from(code)).expect("refusal")
}

/// The MCP initialize error. Its message is the refusal's sentence and its data the refusal.
pub(super) fn mcp_error(refusal: &Refusal) -> McpError {
    let data = serde_json::to_value(refusal).expect("refusal");
    McpError::internal_error(refusal.to_string(), Some(data))
}

pub(super) fn say_line(tape: &TapeRecord, replay: bool) -> String {
    PlayerFrame::Say {
        from: tape.from.clone(),
        address: tape.address.clone(),
        text: tape.text.clone(),
        replay,
    }
    .to_line()
}

pub(super) fn say_value(tape: &TapeRecord) -> Value {
    serde_json::to_value(GmEvent::Say(gm_say(tape))).expect("say")
}

pub(super) fn say_event(tape: &TapeRecord) -> Vec<u8> {
    serde_json::to_vec(&GmEvent::Say(gm_say(tape))).expect("say event")
}

fn gm_say(tape: &TapeRecord) -> SayEvent {
    SayEvent {
        seq: tape.seq,
        from: tape.from.clone(),
        address: tape.address.clone(),
        place: tape.place.clone(),
        actor: tape.actor.clone(),
        recipients: tape.recipients.clone(),
        text: tape.text.clone(),
    }
}

pub(super) fn queued_line(id: &str) -> String {
    PlayerFrame::Queued(Queued { id: id.to_string() }).to_line()
}

pub(super) fn queued_event(queued: &QueuedMove) -> Vec<u8> {
    serde_json::to_vec(&GmEvent::Queued(queued.clone())).expect("queued event")
}

pub(super) fn check_line(outcome: Outcome) -> String {
    PlayerFrame::Check { outcome }.to_line()
}

pub(super) fn check_event(out: &Roll) -> Vec<u8> {
    serde_json::to_vec(&GmEvent::Check(out.clone())).expect("check event")
}

/// The `check` and `refused` lines of this actor's moves.
pub(super) fn check_lines(reports: &[MoveReport], actor_id: &str) -> String {
    let mut out = String::new();
    for report in reports {
        if report.actor_id() != actor_id {
            continue;
        }
        match report {
            MoveReport::Rolled { roll: rolled, .. } => out.push_str(&check_line(rolled.outcome)),
            MoveReport::Refused { code, missing, .. } => out.push_str(&refused_line(Refusal {
                code: protocol::Code::Known(*code),
                detail: missing.clone().map(Detail::One),
            })),
        }
    }
    out
}

pub(super) fn check_events(reports: &[MoveReport]) -> Vec<Vec<u8>> {
    rolls(reports).map(check_event).collect()
}

fn rolls<'a>(reports: impl IntoIterator<Item = &'a MoveReport>) -> impl Iterator<Item = &'a Roll> {
    reports.into_iter().filter_map(|report| match report {
        MoveReport::Rolled { roll: out, .. } => Some(out),
        MoveReport::Refused { .. } => None,
    })
}

pub(super) fn resume_line(view: &engine::View) -> String {
    let actor_id = view.viewer.actor_id();
    let notes = view
        .notes
        .iter()
        .filter(|note| note.to.iter().any(|to| to == actor_id))
        .map(note_view)
        .collect();
    PlayerFrame::Resume {
        notes,
        clocks: view.clocks.clone(),
    }
    .to_line()
}

pub(super) fn text_line(text: &str) -> String {
    PlayerFrame::Lines {
        text: text.to_string(),
    }
    .to_line()
}

pub(super) fn view_line(view: &engine::View) -> String {
    player_view(view).to_line()
}

pub(super) fn perceive_value(view: &engine::View) -> Value {
    serde_json::to_value(player_view(view)).expect("perceive")
}

pub(super) fn commit_value(
    state_rev: u64,
    changed_keys: &[String],
    journal_path: Option<&str>,
    outputs: &[RoundOutput],
) -> Value {
    let checks = rolls(outputs.iter().flat_map(|output| &output.reports))
        .cloned()
        .collect();
    let dice = outputs
        .iter()
        .flat_map(|output| output.dice.iter().cloned())
        .collect();
    let body = CommitBody {
        state_rev,
        changed_keys: changed_keys.to_vec(),
        journal_path: journal_path.map(str::to_string),
        rolls: checks,
        dice,
    };
    serde_json::to_value(body).expect("commit")
}

pub(super) fn status_value(
    now: &State,
    presence: &Presence,
    queued: &[Intent],
    last_seq: u64,
    events: Option<&[TapeRecord]>,
    pack: Option<PackSummary>,
) -> Value {
    let Ok(Value::String(presence_clocks)) = serde_json::to_value(now.pause.presence_clocks) else {
        unreachable!("a run state serializes as its snake_case name");
    };
    let status = TableStatus {
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
        last_seq,
        events: events.map(|events| events.iter().map(gm_say).collect()),
        pack,
    };
    serde_json::to_value(status).expect("table status")
}

fn player_view(view: &engine::View) -> PlayerFrame {
    PlayerFrame::View(Box::new(protocol::View {
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

fn note_view(note: &Note) -> NoteView {
    NoteView {
        id: note.id.clone(),
        to: note.to.clone(),
        place: note.place.clone(),
        fact_ids: note.fact_ids.clone(),
        text: note.text.clone(),
    }
}
