//! What the model reads first on a turn: the server's state as this client has read it, and the
//! observation built from it.

use std::collections::VecDeque;

use serde::Serialize;

use crate::model::Message;
use crate::protocol::{
    MenuRow, PackSummary, QueuedMove, Say, ServerBody, ServerMessage, Speaker, TableStatus,
};
use crate::turn::Turn;

/// How many tape lines the prompt's `tape_lines` keeps.
const RECENT_LINES: usize = 16;

/// The server's state as its messages have shown this client so far, and the turns they
/// wake.
#[derive(Default)]
pub(crate) struct ClientState {
    pub(crate) pack: PackSummary,
    /// The actors present, as of the last message read.
    actors_present: Vec<String>,
    /// The queued moves, as of the last message read.
    pub(crate) queued_moves: Vec<QueuedMove>,
    /// Player says read since a turn last showed them to the model.
    new_player_lines: Vec<TapeLine>,
    wakes: VecDeque<Wake>,
    /// The recent tape every observation shows.
    tape_lines: Vec<TapeLine>,
}

impl ClientState {
    /// Fills the state from the first `status`: the pack, the actors present and the queue,
    /// which wakes a turn when a move already waits.
    pub(crate) fn initialize(&mut self, status: TableStatus) {
        self.pack = status.pack;
        self.actors_present = status.presence.actors_present;
        self.queued_moves = status.queued;
        if !self.queued_moves.is_empty() && !self.wakes.contains(&Wake::QueuedMove) {
            self.wakes.push_front(Wake::QueuedMove);
        }
    }

    /// Records what one server message says about the game: a player's say wakes a turn, the
    /// GM's own say (the reply to its command) joins `tape_lines`, a queued move joins the
    /// queued moves and wakes a turn, a round empties them, and presence names the actors
    /// present.
    pub(crate) fn record_server_message(&mut self, message: &ServerMessage) {
        let seq = message.seq.unwrap_or_default();
        match &message.body {
            ServerBody::Say(say) if matches!(say.from, Speaker::Player(_)) => {
                let line = TapeLine::new(seq, say.clone());
                self.new_player_lines.push(line.clone());
                self.wakes.push_back(Wake::Say(line));
            }
            ServerBody::Say(say) if say.from == Speaker::Gm => {
                push_recent(&mut self.tape_lines, TapeLine::new(seq, say.clone()));
            }
            ServerBody::Queued {
                intent,
                actor,
                move_name,
                ..
            } => {
                self.queued_moves.push(QueuedMove {
                    id: intent.clone(),
                    actor_id: actor.clone(),
                    move_name: move_name.clone(),
                });
                if !self.wakes.contains(&Wake::QueuedMove) {
                    self.wakes.push_back(Wake::QueuedMove);
                }
            }
            ServerBody::Round { .. } => {
                self.queued_moves.clear();
                self.wakes.retain(|wake| *wake != Wake::QueuedMove);
            }
            ServerBody::Presence { actors_present } => {
                self.actors_present = actors_present.clone();
            }
            _ => {}
        }
    }

    /// The next turn, when a wake waits: its observation shows the new player lines, which
    /// then join `tape_lines`, and a say the model has now read wakes no turn.
    pub(crate) fn start_turn(&mut self) -> Option<Turn> {
        let wake = self.wakes.pop_front()?;
        let prompt = render_prompt(&Observation {
            menu: &self.pack.menu,
            tape_lines: &self.tape_lines,
            new_player_lines: &self.new_player_lines,
            queued_moves: &self.queued_moves,
            wake: &wake,
            actors_present: &self.actors_present,
        });
        for line in std::mem::take(&mut self.new_player_lines) {
            push_recent(&mut self.tape_lines, line);
        }
        self.wakes.retain(|wake| *wake == Wake::QueuedMove);
        Some(Turn {
            messages: vec![Message::User { content: prompt }],
        })
    }
}

/// One say as the model reads it in the prompt's `tape_lines` and `new_player_lines`, with the `seq` of the
/// message that carried it and without `recipients`. The GM's own lines carry the `seq` of
/// the server's reply.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct TapeLine {
    pub(crate) seq: u64,
    from: Speaker,
    address: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    place: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    actor: Option<String>,
    text: String,
}

impl TapeLine {
    /// The say a message with `seq` carried.
    pub(crate) fn new(seq: u64, say: Say) -> Self {
        Self {
            seq,
            from: say.from,
            address: say.address,
            place: say.place,
            actor: say.actor,
            text: say.text,
        }
    }
}

/// What starts a turn: a player's say, or a queued move.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Wake {
    Say(TapeLine),
    QueuedMove,
}

/// Pushes `line` onto the prompt's `tape_lines`, and drops the oldest past `RECENT_LINES`.
fn push_recent(tape_lines: &mut Vec<TapeLine>, line: TapeLine) {
    tape_lines.push(line);
    if tape_lines.len() > RECENT_LINES {
        tape_lines.drain(..tape_lines.len() - RECENT_LINES);
    }
}

/// What the GM can see of the game when a turn starts, sent as the turn's first message on
/// every model call: the menu, the recent tape, the new player lines, the queued moves, what
/// woke the turn and the actors present.
#[derive(Serialize)]
struct Observation<'a> {
    menu: &'a [MenuRow],
    tape_lines: &'a [TapeLine],
    new_player_lines: &'a [TapeLine],
    queued_moves: &'a [QueuedMove],
    wake: &'a Wake,
    actors_present: &'a [String],
}

fn render_prompt(observation: &Observation) -> String {
    serde_json::to_string(observation).expect("prompt json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tape_lines_keep_sixteen_and_drop_the_oldest() {
        let mut tape_lines = Vec::new();
        for index in 0..20 {
            let line = TapeLine {
                seq: index,
                from: Speaker::Player("rowan".to_string()),
                address: "table".to_string(),
                place: None,
                actor: None,
                text: index.to_string(),
            };
            push_recent(&mut tape_lines, line);
        }
        assert_eq!(tape_lines.len(), RECENT_LINES);
        assert_eq!(tape_lines[0].text, "4");
        assert_eq!(tape_lines[15].text, "19");
    }
}
