//! What a round writes down.

use crate::rules::effect::Effect;
use crate::rules::gate;
use crate::rules::roll::{MoveReport, Roll};
use crate::store::records::{Arrival, Step};

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
