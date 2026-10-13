use spookery_kernel::world::Draft;
use spookery_kernel::Reason;

use crate::round::Round;

/// Trigger rows land here in R3.
pub fn run_triggers(round: &mut Round, _draft: &mut Draft) -> Result<(), Reason> {
    round.trace.push("run_triggers".to_string());
    Ok(())
}
