//! What a round writes down, and the round's half of a journal line.

use crate::effect::Effect;
use crate::intent::Intent;
use crate::journal::{Arrival, Rejected, Step};
use crate::model::Presence;
use crate::round::stage::advance::Clock;
use crate::rules::MoveReport;

/// Everything a round writes down, in the order it happened. `record` consumes it.
#[derive(Default)]
pub(crate) struct RoundLog {
    pub rejected: Vec<Rejected>,
    pub effects: Vec<Effect>,
    pub arrivals: Vec<Arrival>,
    pub dice: Vec<String>,
    /// The journal's `steps_run`.
    pub steps: Vec<Step>,
    pub reports: Vec<MoveReport>,
}

/// The round's half of a journal line. `store::commit` seals it into a `RoundRecord`.
pub(crate) struct RoundLine {
    pub clock: Clock,
    pub presence: Presence,
    pub seed: String,
    pub wall: String,
    pub fast_forward: bool,
    pub log: RoundLog,
    /// The journal's `intents`: the queue the round was given.
    pub queue: Vec<Intent>,
}
