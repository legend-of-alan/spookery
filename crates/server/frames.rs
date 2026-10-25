//! Player lines, `spook/event` frames, and tool results.

use crate::protocol::{
    CommitBody, GmSay, Outcome, OutgoingSay, PlayerMessage, QueueRow, Roll, SessionStatus,
    StatusPause, StatusPresence, ViewScene, WireClock, WireFact, WireItem, WireMessage, WireMeter,
};
use serde_json::{Map, Value};
use spookery_engine::model::{Message, Now, RunState};
use spookery_engine::store::{record_line, TapeRecord};
use spookery_engine::{
    ClockView, ExitView, Heard, Intent, ItemStack, KnownFact, MeterView, MoveReport, Reason,
    RollResult, RoundOutput, View, World,
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

pub fn pack_event(world: &World) -> Vec<u8> {
    crate::protocol::write_pack_event(&world.pack_summary())
}

pub fn player_check_line(outcome: Outcome) -> String {
    crate::protocol::write_player(&PlayerMessage::Check { outcome })
}

pub fn check_event(out: &RollResult) -> Vec<u8> {
    crate::protocol::write_check_event(&roll_of(out))
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
            MoveReport::Refused { code, missing, .. } => {
                out.push_str(&player_refused(code.token(), missing.as_deref()))
            }
        }
    }
    out
}

pub fn gm_check_frames(reports: &[MoveReport]) -> Vec<Vec<u8>> {
    reports
        .iter()
        .filter_map(|report| match report {
            MoveReport::Rolled { out, .. } => Some(check_event(out)),
            MoveReport::Refused { .. } => None,
        })
        .collect()
}

pub fn player_resume_line(view: &View) -> String {
    let actor_id = view.viewer.actor_id();
    let messages = view
        .messages
        .iter()
        .filter(|message| message.to.iter().any(|to| to == actor_id))
        .map(wire_message)
        .collect();
    let clocks = view.clocks.iter().map(wire_clock).collect();
    crate::protocol::write_player(&PlayerMessage::Resume { messages, clocks })
}

pub fn player_lines(text: &str) -> String {
    crate::protocol::write_player(&PlayerMessage::Lines {
        text: text.to_string(),
    })
}

pub fn player_refused(code: &str, missing: Option<&str>) -> String {
    crate::protocol::write_player(&PlayerMessage::Refused {
        code: code.to_string(),
        missing: missing.map(|text| crate::protocol::Missing::One(text.to_string())),
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
    let checks = outputs
        .iter()
        .flat_map(|output| output.reports.iter())
        .filter_map(|report| match report {
            MoveReport::Rolled { out, .. } => Some(roll_of(out)),
            MoveReport::Refused { .. } => None,
        })
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
        tick_index: now.tick_index,
        state_rev: now.state_rev,
        pack_rev: now.pack_rev.clone(),
        last_tick_at: now.last_tick_at.clone(),
        queued: queued
            .iter()
            .map(|intent| QueueRow {
                id: intent.id.clone(),
                actor_id: intent.actor_id.clone(),
                move_name: intent.move_name.clone(),
            })
            .collect(),
        last_seq,
        events: events.map(|events| events.iter().map(gm_say).collect()),
    }
    .to_value()
}

fn player_view(view: &View) -> PlayerMessage {
    PlayerMessage::View {
        viewer: view.viewer.token(),
        place: view.place.clone(),
        scene: Box::new(ViewScene {
            place_name: view.place_name.clone(),
            prose: view.prose.clone(),
            actor_names: view.actor_names.clone(),
            status: view.status.clone(),
        }),
        visible_actors: view.visible_actors.clone(),
        known_facts: view.known_facts.iter().map(wire_fact).collect(),
        messages: view.messages.iter().map(wire_message).collect(),
        unknown_mentioned: view.unknown_mentioned.clone(),
        clocks: view.clocks.iter().map(wire_clock).collect(),
        items: view.items.iter().map(wire_item).collect(),
        meters: view.meters.iter().map(wire_meter).collect(),
        habits: view.habits.clone(),
        exits: view.exits.iter().map(wire_exit).collect(),
    }
}

fn wire_exit(exit: &ExitView) -> crate::protocol::WireExit {
    crate::protocol::WireExit {
        name: exit.name.clone(),
        to: exit.to.clone(),
        place_name: exit.place_name.clone(),
        minutes: exit.minutes,
        locked: exit.locked,
    }
}

fn wire_fact(fact: &KnownFact) -> WireFact {
    WireFact {
        id: fact.id.clone(),
        text: fact.text.clone(),
        body: fact.body.clone(),
    }
}

fn wire_message(message: &Message) -> WireMessage {
    WireMessage {
        id: message.id.clone(),
        to: message.to.clone(),
        place: message.place.clone(),
        fact_ids: message.fact_ids.clone(),
        text: message.text.clone(),
    }
}

fn wire_clock(clock: &ClockView) -> WireClock {
    WireClock {
        id: clock.id.clone(),
        value: clock.value,
        max: clock.max,
        paused: clock.paused,
    }
}

fn wire_item(item: &ItemStack) -> WireItem {
    WireItem {
        id: item.id.clone(),
        count: item.count,
    }
}

fn wire_meter(meter: &MeterView) -> WireMeter {
    WireMeter {
        id: meter.id.clone(),
        value: meter.value,
        max: meter.max,
    }
}

fn roll_of(out: &RollResult) -> Roll {
    Roll {
        faces: out.faces.clone(),
        stat: out.stat,
        total: out.total,
        target: out.target,
        outcome: out.outcome,
    }
}

fn run_state(state: RunState) -> String {
    match state {
        RunState::Paused => "paused",
        RunState::Running => "running",
    }
    .to_string()
}
