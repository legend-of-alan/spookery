use spookery_kernel::world::Draft;
use spookery_kernel::Reason;

use crate::round::Round;

/// Collect roll notices for viewers. No new prose.
pub(crate) fn run(round: &mut Round, _draft: &mut Draft) -> Result<(), Reason> {
    round.trace.push("observe".to_string());
    round.notices.append(&mut round.roll_reports);
    Ok(())
}
