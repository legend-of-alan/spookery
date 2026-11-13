//! What the model reads first on a turn: the server's state as this client has read it, and the
//! opening prompt built from it.

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
    /// The recent tape every opening prompt shows.
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

    /// What one message says about the server's state: a player's say wakes a turn, a queued
    /// move joins the queued moves and wakes one, a round empties them, and presence names the
    /// actors present.
    pub(crate) fn record(&mut self, message: &ServerMessage) {
        match &message.body {
            ServerBody::Say(say) if matches!(say.from, Speaker::Player(_)) => {
                let line = TapeLine::new(message.seq.unwrap_or_default(), say.clone());
                self.new_player_lines.push(line.clone());
                self.wakes.push_back(Wake::Say(line));
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

    /// The next turn, when a wake waits: its opening prompt and the new player lines.
    pub(crate) fn start_turn(&mut self) -> Option<Turn> {
        let wake = self.wakes.pop_front()?;
        let player_lines = std::mem::take(&mut self.new_player_lines);
        let prompt = render_prompt(&Opening {
            menu: &self.pack.menu,
            tape_lines: &self.tape_lines,
            new_player_lines: &player_lines,
            queued_moves: &self.queued_moves,
            wake: &wake,
            actors_present: &self.actors_present,
        });
        Some(Turn {
            messages: vec![Message::User { content: prompt }],
            player_lines,
            gm_lines: Vec::new(),
        })
    }

    /// Records a played turn: its player and GM lines join `tape_lines`, and a say the model
    /// has now read wakes no turn.
    pub(crate) fn record_turn(&mut self, turn: Turn) {
        let seen = turn.player_lines.iter().map(|line| line.seq).max();
        for line in turn.player_lines.into_iter().chain(turn.gm_lines) {
            keep_recent(&mut self.tape_lines, line);
        }
        if let Some(seen) = seen {
            self.wakes.retain(|wake| match wake {
                Wake::Say(line) => line.seq > seen,
                Wake::QueuedMove => true,
            });
        }
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

/// Add `line` to the prompt's `tape_lines`, which keeps the latest `RECENT_LINES`.
fn keep_recent(tape_lines: &mut Vec<TapeLine>, line: TapeLine) {
    tape_lines.push(line);
    if tape_lines.len() > RECENT_LINES {
        tape_lines.drain(..tape_lines.len() - RECENT_LINES);
    }
}

/// What the model reads first on a turn: the menu, the recent tape, the new player lines,
/// the queued moves, what woke the turn and the actors present.
#[derive(Serialize)]
struct Opening<'a> {
    menu: &'a [MenuRow],
    tape_lines: &'a [TapeLine],
    new_player_lines: &'a [TapeLine],
    queued_moves: &'a [QueuedMove],
    wake: &'a Wake,
    actors_present: &'a [String],
}

fn render_prompt(opening: &Opening) -> String {
    serde_json::to_string(opening).expect("prompt json")
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
            keep_recent(&mut tape_lines, line);
        }
        assert_eq!(tape_lines.len(), RECENT_LINES);
        assert_eq!(tape_lines[0].text, "4");
        assert_eq!(tape_lines[15].text, "19");
    }
}
