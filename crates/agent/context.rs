//! What the model reads first on a turn: the server's state as this client has read it, and the
//! observation built from it.

use std::collections::VecDeque;

use serde::Serialize;

use crate::model;
use crate::protocol::{
    Audience, MenuRow, PackSummary, QueuedMove, ServerBody, ServerMessage, Speaker, Speech,
    TableStatus,
};
use crate::turn::Turn;

/// How many tape lines the prompt's `tape_lines` keeps.
const RECENT_LINES: usize = 16;

/// How to be the GM, the same for every pack: the system message starts with it, and the
/// pack's `gm_notes` follow.
pub(crate) const GM_INSTRUCTIONS: &str = "\
You are the game master. You can't change the game state. Only the engine changes it, by applying the pack's rules and rolling its dice. You can look at the world, speak, and propose moves, and the engine decides what happens.

Your goal is to keep the players engaged and fairly challenged. Don't help them or make things easier unless they seem stuck with no way forward. Make the world feel alive and a little unpredictable: NPCs have their own aims and act on them, and small surprises happen. Keep it believable, never chaotic.

Each turn starts with the game state as JSON: the moves on offer, recent lines, what players just said, the moves queued so far, what started the turn, and who is present.

- Look through the eyes of a player's character or an NPC before you describe their surroundings, and act on what you see.
- Narrate, and speak for NPCs. You can't move or speak for a player's character, so don't narrate their thoughts, words, or actions either. Describe only what they actually did.
- Propose moves for NPCs from the moves on offer. A refused move did not happen.
- When every player present has a move queued, end the turn. If someone hasn't moved yet, wait for them.
- Narrate what the rolls say happened. A failed roll costs something or complicates things. It never leaves the story stuck.
- Add mood and description, but never invent people, items, exits, or outcomes the engine didn't give you, and never contradict the game state or the story so far.
- Under no circumstances reveal anything a player's character hasn't perceived. That includes what an NPC thinks or plans: narrate only what an NPC says or visibly does.
- End your narration on the situation in front of the players, so they know it's their move.
- When you are done, answer with no tool calls.";

/// The server's state as its messages have shown this client so far, and the turns they
/// start.
#[derive(Default)]
pub(crate) struct ClientState {
    pub(crate) pack: PackSummary,
    /// The actors present, as of the last message read.
    actors_present: Vec<String>,
    /// The queued moves, as of the last message read.
    pub(crate) queued_moves: Vec<QueuedMove>,
    /// Player speech read since a turn last showed it to the model.
    new_player_lines: Vec<TapeLine>,
    causes: VecDeque<TurnCause>,
    /// The recent tape every observation shows.
    tape_lines: Vec<TapeLine>,
}

impl ClientState {
    /// Fills the state from the first `status`: the pack, the actors present and the queue,
    /// which starts a turn when a move already waits.
    pub(crate) fn initialize(&mut self, status: TableStatus) {
        self.pack = status.pack;
        self.actors_present = status.presence.actors_present;
        self.queued_moves = status.queued;
        self.add_queued_move_cause(VecDeque::push_front);
    }

    /// Records what one server message shows about the game: a player's speech starts a turn,
    /// the GM's own speech (the reply to its `say`, as the GM or an NPC) joins `tape_lines`,
    /// a queued move joins the queued moves and starts a turn, a round empties them, and
    /// presence names the actors present.
    pub(crate) fn record_server_message(&mut self, message: &ServerMessage) {
        let seq = message.seq.unwrap_or_default();
        match &message.body {
            ServerBody::Say(speech) if matches!(speech.from, Speaker::Player(_)) => {
                self.new_player_lines
                    .push(TapeLine::new(seq, speech.clone()));
                self.causes.push_back(TurnCause::Speech);
            }
            ServerBody::Say(speech) if matches!(speech.from, Speaker::Gm | Speaker::Npc(_)) => {
                push_recent(&mut self.tape_lines, TapeLine::new(seq, speech.clone()));
            }
            ServerBody::Queued {
                intent,
                actor_id,
                move_id,
                ..
            } => {
                self.queued_moves.push(QueuedMove {
                    id: intent.clone(),
                    actor_id: actor_id.clone(),
                    move_id: move_id.clone(),
                });
                self.add_queued_move_cause(VecDeque::push_back);
            }
            ServerBody::Round { .. } => {
                self.queued_moves.clear();
                self.causes.retain(|cause| *cause != TurnCause::QueuedMove);
            }
            ServerBody::Presence { actors_present } => {
                self.actors_present = actors_present.clone();
            }
            _ => {}
        }
    }

    /// Adds the queued-move turn cause with `push` when a move is queued and no queued-move
    /// cause waits yet.
    fn add_queued_move_cause(&mut self, push: fn(&mut VecDeque<TurnCause>, TurnCause)) {
        if !self.queued_moves.is_empty() && !self.causes.contains(&TurnCause::QueuedMove) {
            push(&mut self.causes, TurnCause::QueuedMove);
        }
    }

    /// The next turn and its cause, when a turn cause waits: it opens with the system message,
    /// then the observation, which shows the new player lines. Those then join `tape_lines`, and
    /// a speech the model has now read starts no turn.
    pub(crate) fn start_turn(&mut self) -> Option<(TurnCause, Turn)> {
        let cause = self.causes.pop_front()?;
        let prompt = serde_json::to_string(&Observation {
            menu: &self.pack.menu,
            tape_lines: &self.tape_lines,
            new_player_lines: &self.new_player_lines,
            queued_moves: &self.queued_moves,
            cause: &cause,
            actors_present: &self.actors_present,
        })
        .expect("prompt json");
        for line in std::mem::take(&mut self.new_player_lines) {
            push_recent(&mut self.tape_lines, line);
        }
        self.causes.retain(|cause| *cause == TurnCause::QueuedMove);
        let conversation = vec![
            model::Message::System {
                content: self.system_text(),
            },
            model::Message::User { content: prompt },
        ];
        Some((cause, Turn { conversation }))
    }

    /// `GM_INSTRUCTIONS`, then the pack's `gm_notes` after a blank line when it has one.
    fn system_text(&self) -> String {
        match self.pack.gm_notes.as_str() {
            "" => GM_INSTRUCTIONS.to_string(),
            notes => format!("{GM_INSTRUCTIONS}\n\n{notes}"),
        }
    }
}

/// One line of speech as the model reads it in the prompt's `tape_lines` and
/// `new_player_lines`, with the `seq` of the message that carried it and without `recipient_ids`.
/// The GM's own lines carry the `seq` of the server's reply.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct TapeLine {
    pub(crate) seq: u64,
    from: Speaker,
    to: Audience,
    text: String,
}

impl TapeLine {
    /// The speech a message with `seq` carried.
    pub(crate) fn new(seq: u64, speech: Speech) -> Self {
        Self {
            seq,
            from: speech.from,
            to: speech.to,
            text: speech.text,
        }
    }
}

/// What starts a turn: a player's speech, which the prompt shows in `new_player_lines`, or a
/// queued move.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TurnCause {
    Speech,
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
/// started the turn and the actors present.
#[derive(Serialize)]
struct Observation<'a> {
    menu: &'a [MenuRow],
    tape_lines: &'a [TapeLine],
    new_player_lines: &'a [TapeLine],
    queued_moves: &'a [QueuedMove],
    cause: &'a TurnCause,
    actors_present: &'a [String],
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
                to: Audience::Table,
                text: index.to_string(),
            };
            push_recent(&mut tape_lines, line);
        }
        assert_eq!(tape_lines.len(), RECENT_LINES);
        assert_eq!(tape_lines[0].text, "4");
        assert_eq!(tape_lines[15].text, "19");
    }
}
