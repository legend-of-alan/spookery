//! What a round writes down.

use crate::rules::effect::Effect;
use crate::rules::gate;
use crate::rules::roll::{MoveReport, Roll};
use crate::state::change::Change;
use crate::state::id::ActorId;
use crate::store::records::Step;

/// Everything a round writes down, in the order it happened. `record` consumes it.
#[derive(Default)]
pub(crate) struct RoundLog {
    pub refused: Vec<gate::Refusal>,
    pub effects: Vec<Effect>,
    pub dice: Vec<String>,
    pub extra_steps: Vec<Step>,
    pub reports: Vec<MoveReport>,
    /// Every change record of the round, trigger pass by trigger pass.
    pub changes: Vec<Change>,
}

impl RoundLog {
    /// Tell the actor the roll their rule made, if it made one.
    pub(crate) fn report_roll(&mut self, rolled: Option<Roll>, actor_id: Option<&ActorId>) {
        self.reports.extend(
            rolled
                .zip(actor_id)
                .map(|(roll, actor_id)| MoveReport::Rolled {
                    actor_id: actor_id.clone(),
                    roll,
                }),
        );
    }
}
