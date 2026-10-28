//! Player lines, `spook/event` frames, and tool results.

use crate::protocol::{
    CommitBody, GmSay, MessageView, Missing, Outcome, OutgoingSay, PackSummary, PlayerMessage,
    QueueRow, SessionStatus, StatusPause, StatusPresence, ViewScene,
};
use serde_json::{Map, Value};
use spookery_engine::{
    record_line, Heard, Intent, Message, MoveReport, Now, Reason, Roll, RoundOutput, RunState,
    TapeRecord, View, World,
};

#[derive(Debug)]
pub struct Spoken {
    pub tape: String,
    pub player_event: String,
    pub gm_event: Vec<u8>,
    pub tool: Value,
    pub recipients: Vec<String>,
}

/// GM `say`. `from` is the seat. A supplied `from` appends nothing.
pub fn speak_tool(world: &World, arguments: Option<Map<String, Value>>) -> Result<Spoken, Reason> {
    let say = crate::protocol::read_say_args(arguments.as_ref()).map_err(|_| Reason::Schema)?;
    let heard = match &say {
        OutgoingSay::Table { .. } => Heard::Table,
        OutgoingSay::Place { place, .. } => Heard::Place(place),
        OutgoingSay::Actor { actor, .. } => Heard::Actor(actor),
    };
    let stored = world.speak_gm(heard, say.text())?;
    Ok(spoken_from(&stored))
}

pub(super) fn spoken_from(stored: &TapeRecord) -> Spoken {
    Spoken {
        tape: record_line(stored),
        player_event: player_say_line(stored),
        gm_event: say_event(stored),
        tool: say_value(stored),
        recipients: stored.recipients.clone(),
    }
}

pub fn player_say_line(tape: &TapeRecord) -> String {
    say_line(tape, false)
}

pub fn player_replay_line(tape: &TapeRecord) -> String {
    say_line(tape, true)
}

fn say_line(tape: &TapeRecord, replay: bool) -> String {
    crate::protocol::write_player(&PlayerMessage::Say {
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
    crate::protocol::write_say_event(&gm_say(tape))
}

fn gm_say(tape: &TapeRecord) -> GmSay {
    GmSay {
        seq: tape.seq,
        from: tape.from.clone(),
        address: tape.address.clone(),
        place: tape.place.clone(),
        actor: tape.actor.clone(),
        recipients: tape.recipients.clone(),
        text: tape.text.clone(),
    }
}

pub fn player_queued_line(id: &str) -> String {
    crate::protocol::write_player(&PlayerMessage::Queued { id: id.to_string() })
}

pub fn queued_event(id: &str, actor_id: &str, move_name: &str) -> Vec<u8> {
    crate::protocol::write_queued_event(id, actor_id, move_name)
}

pub fn player_check_line(outcome: Outcome) -> String {
    crate::protocol::write_player(&PlayerMessage::Check { outcome })
}

pub fn check_event(out: &Roll) -> Vec<u8> {
    crate::protocol::write_check_event(out)
}

pub fn player_report_lines(reports: &[MoveReport], actor_id: &str) -> String {
    let mut out = String::new();
    for report in reports {
        if report.actor_id() != actor_id {
            continue;
        }
        match report {
            MoveReport::Rolled { out: rolled, .. } => {
                out.push_str(&player_check_line(rolled.outcome))
            }
            MoveReport::Refused { code, missing, .. } => out.push_str(&player_refused(
                code.token(),
                missing.as_ref().map(|text| Missing::One(text.clone())),
            )),
        }
    }
    out
}

pub fn gm_check_frames(reports: &[MoveReport]) -> Vec<Vec<u8>> {
    rolls(reports).map(check_event).collect()
}

fn rolls<'a>(reports: impl IntoIterator<Item = &'a MoveReport>) -> impl Iterator<Item = &'a Roll> {
    reports.into_iter().filter_map(|report| match report {
        MoveReport::Rolled { out, .. } => Some(out),
        MoveReport::Refused { .. } => None,
    })
}

pub fn player_resume_line(view: &View) -> String {
    let actor_id = view.viewer.actor_id();
    let messages = view
        .messages
        .iter()
        .filter(|message| message.to.iter().any(|to| to == actor_id))
        .map(message_view)
        .collect();
    crate::protocol::write_player(&PlayerMessage::Resume {
        messages,
        clocks: view.clocks.clone(),
    })
}

pub fn player_lines(text: &str) -> String {
    crate::protocol::write_player(&PlayerMessage::Lines {
        text: text.to_string(),
    })
}

pub fn player_refused(code: &str, missing: Option<Missing>) -> String {
    crate::protocol::write_player(&PlayerMessage::Refused {
        code: code.to_string(),
        missing,
    })
}

pub fn player_perceive_line(view: &View) -> String {
    crate::protocol::write_player(&player_view(view))
}

pub fn perceive_value(view: &View) -> Value {
    serde_json::to_value(player_view(view)).expect("perceive")
}

pub fn commit_reply(
    state_rev: u64,
    diff_summary: &[String],
    log_path: Option<&str>,
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
        diff_summary: diff_summary.to_vec(),
        log_path: log_path.map(str::to_string),
        checks,
        dice,
    }
    .to_value()
}

pub fn session_status(
    now: &Now,
    queued: &[Intent],
    last_seq: u64,
    events: Option<&[TapeRecord]>,
    pack: Option<PackSummary>,
) -> Value {
    SessionStatus {
        presence: StatusPresence {
            humans: now.presence.humans,
            actors_present: now.presence.actors_present.clone(),
        },
        pause: StatusPause {
            named_plot_clocks: run_state(now.pause.named_plot_clocks),
        },
        world_time: now.world_time,
        tick_index: now.round_index,
        state_rev: now.state_rev,
        pack_rev: now.pack_rev.clone(),
        last_tick_at: now.last_round_at.clone(),
        queued: queued
            .iter()
            .map(|intent| QueueRow {
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

fn player_view(view: &View) -> PlayerMessage {
    PlayerMessage::View {
        viewer: view.viewer.actor_id().to_string(),
        place: view.place.clone(),
        scene: Box::new(ViewScene {
            place_name: view.place_name.clone(),
            prose: view.prose.clone(),
            actor_names: view.actor_names.clone(),
            status: view.status.clone(),
        }),
        visible_actors: view.visible_actors.clone(),
        known_facts: view.known_facts.clone(),
        messages: view.messages.iter().map(message_view).collect(),
        unknown_mentioned: view.unknown_mentioned.clone(),
        clocks: view.clocks.clone(),
        items: view.items.clone(),
        meters: view.meters.clone(),
        habits: view.habits.clone(),
        exits: view.exits.clone(),
    }
}

fn message_view(message: &Message) -> MessageView {
    MessageView {
        id: message.id.clone(),
        to: message.to.clone(),
        place: message.place.clone(),
        fact_ids: message.fact_ids.clone(),
        text: message.text.clone(),
    }
}

fn run_state(state: RunState) -> String {
    match state {
        RunState::Paused => "paused",
        RunState::Running => "running",
    }
    .to_string()
}
