use spookery_kernel::world::Draft;
use spookery_kernel::Reason;

use crate::round::Round;

/// Arrivals land here in W1.
pub fn complete(round: &mut Round, _draft: &mut Draft) -> Result<(), Reason> {
    round.trace.push("complete".to_string());
    Ok(())
}
