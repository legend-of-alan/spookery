//! What a round writes down, and the round's half of a journal line.

use crate::effect::Effect;
use crate::intent::Intent;
use crate::journal::{Arrival, Step};
use crate::model::Presence;
use crate::round::stage::advance::RoundTime;
use crate::rules::gate;
use crate::rules::{MoveReport, Roll};

/// Everything a round writes down, in the order it happened. `record` consumes it.
#[derive(Default)]
pub(crate) struct RoundLog {
    pub refused: Vec<gate::Refusal>,
    pub effects: Vec<Effect>,
    pub arrivals: Vec<Arrival>,
    pub dice: Vec<String>,
    pub extra_steps: Vec<Step>,
    pub reports: Vec<MoveReport>,
}

impl RoundLog {
    /// Tell the actor the roll their rule made, if it made one.
    pub(crate) fn report_roll(&mut self, rolled: Option<Roll>, actor_id: &str) {
        self.reports.extend(rolled.map(|roll| MoveReport::Rolled {
            actor_id: actor_id.to_string(),
            roll,
        }));
    }
}

/// The round's half of a journal line. `store::commit` seals it into a `RoundRecord`.
pub(crate) struct PlayedRound {
    pub clock: RoundTime,
    pub presence: Presence,
    pub seed: String,
    pub last_round_at: String,
    pub fast_forward: bool,
    pub log: RoundLog,
    /// The journal's `intents`: the queue the round was given.
    pub intents: Vec<Intent>,
}
