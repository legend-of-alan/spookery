//! Player lines, `spook/event` frames, and tool results.

use crate::protocol::{
    CommitBody, EventBody, Missing, NoteView, Outcome, PackSummary, PlayerFrame, QueuedMove,
    SayEvent, StatusPause, StatusPresence, TableStatus, ViewScene,
};
use rmcp::ErrorData as McpError;
use serde_json::Value;
use spookery_engine::{
    GateRefusal, Intent, MoveReport, Note, Presence, Reason, Roll, RoundOutput, State, TapeRecord,
    View,
};

/// The one place a refusal becomes bytes: a player line, a GM tool result or an MCP error.
#[derive(Debug)]
pub(crate) struct Refusal {
    code: Reason,
    detail: Option<Missing>,
}

impl From<Reason> for Refusal {
    fn from(code: Reason) -> Self {
        Self { code, detail: None }
    }
}

impl From<&GateRefusal> for Refusal {
    fn from(refusal: &GateRefusal) -> Self {
        Self::short(refusal.reason(), refusal.missing())
    }
}

impl Refusal {
    /// `CHOOSE_PC` names the free PCs.
    pub(crate) fn choose_pc(free: Vec<String>) -> Self {
        Self {
            code: Reason::ChoosePc,
            detail: Some(Missing::Many(free)),
        }
    }

    /// `missing` is the meter or item id, or the sentence a locked exit shows.
    fn short(code: Reason, missing: Option<&str>) -> Self {
        Self {
            code,
            detail: missing.map(|text| Missing::One(text.to_string())),
        }
    }

    /// The player `refused` line. `detail` is left out when there is none.
    pub(crate) fn line(&self) -> String {
        crate::protocol::emit_player(&PlayerFrame::Refused {
            code: self.code.as_str().to_string(),
            detail: self.detail.clone(),
        })
    }

    /// The GM tool result names the code only.
    pub(crate) fn tool_result(&self) -> Value {
        crate::protocol::code_result(self.code.as_str())
    }

    pub(crate) fn mcp_error(&self) -> McpError {
        McpError::internal_error(crate::protocol::refusal(self.code.as_str()), None)
    }
}

pub fn say(tape: &TapeRecord, replay: bool) -> String {
    crate::protocol::emit_player(&PlayerFrame::Say {
        from: tape.from.clone(),
        address: tape.address.clone(),
        text: tape.text.clone(),
        replay,
    })
}

pub fn say_value(tape: &TapeRecord) -> Value {
    gm_say(tape).to_value()
}

pub fn say_event(tape: &TapeRecord) -> Vec<u8> {
    serde_json::to_vec(&EventBody::Say(gm_say(tape))).expect("say event")
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

pub fn queued(id: &str) -> String {
    crate::protocol::emit_player(&PlayerFrame::Queued { id: id.to_string() })
}

pub fn queued_event(id: &str, actor_id: &str, move_name: &str) -> Vec<u8> {
    let row = QueuedMove {
        id: id.to_string(),
        actor_id: actor_id.to_string(),
        move_name: move_name.to_string(),
    };
    serde_json::to_vec(&EventBody::Queued(row)).expect("queued event")
}

pub fn check(outcome: Outcome) -> String {
    crate::protocol::emit_player(&PlayerFrame::Check { outcome })
}

pub fn check_event(out: &Roll) -> Vec<u8> {
    serde_json::to_vec(&EventBody::Check(out.clone())).expect("check event")
}

/// The `check` and `refused` lines of this actor's moves.
pub fn reports(reports: &[MoveReport], actor_id: &str) -> String {
    let mut out = String::new();
    for report in reports {
        if report.actor_id() != actor_id {
            continue;
        }
        match report {
            MoveReport::Rolled { roll: rolled, .. } => out.push_str(&check(rolled.outcome)),
            MoveReport::Refused { code, missing, .. } => {
                out.push_str(&Refusal::short(*code, missing.as_deref()).line())
            }
        }
    }
    out
}

pub fn check_events(reports: &[MoveReport]) -> Vec<Vec<u8>> {
    rolls(reports).map(check_event).collect()
}

fn rolls<'a>(reports: impl IntoIterator<Item = &'a MoveReport>) -> impl Iterator<Item = &'a Roll> {
    reports.into_iter().filter_map(|report| match report {
        MoveReport::Rolled { roll: out, .. } => Some(out),
        MoveReport::Refused { .. } => None,
    })
}

pub fn resume(view: &View) -> String {
    let actor_id = view.viewer.actor_id();
    let notes = view
        .notes
        .iter()
        .filter(|note| note.to.iter().any(|to| to == actor_id))
        .map(note_view)
        .collect();
    crate::protocol::emit_player(&PlayerFrame::Resume {
        notes,
        clocks: view.clocks.clone(),
    })
}

pub fn lines(text: &str) -> String {
    crate::protocol::emit_player(&PlayerFrame::Lines {
        text: text.to_string(),
    })
}

pub fn view(view: &View) -> String {
    crate::protocol::emit_player(&player_view(view))
}

pub fn perceive_value(view: &View) -> Value {
    serde_json::to_value(player_view(view)).expect("perceive")
}

pub fn commit(
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
    CommitBody {
        state_rev,
        changed_keys: changed_keys.to_vec(),
        journal_path: journal_path.map(str::to_string),
        rolls: checks,
        dice,
    }
    .to_value()
}

pub fn status(
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
    TableStatus {
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
    }
    .to_value()
}

fn player_view(view: &View) -> PlayerFrame {
    PlayerFrame::View {
        viewer: view.viewer.actor_id().to_string(),
        place: view.place.clone(),
        scene: Box::new(ViewScene {
            place_name: view.place_name.clone(),
            body: view.prose.clone(),
            actor_names: view.actor_names.clone(),
            status: view.status.clone(),
        }),
        visible_actors: view.visible_actors.clone(),
        known_facts: view.known_facts.clone(),
        notes: view.notes.iter().map(note_view).collect(),
        unknown_mentions: view.unknown_mentions.clone(),
        clocks: view.clocks.clone(),
        items: view.items.clone(),
        meters: view.meters.clone(),
        habits: view.habits.clone(),
        exits: view.exits.clone(),
    }
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
