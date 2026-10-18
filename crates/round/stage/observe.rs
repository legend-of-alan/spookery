//! Deliver narrate lines and build the change summary for each viewer.

use std::collections::BTreeSet;

use spookery_kernel::api::draft::Draft;
use spookery_kernel::api::effect::EffectKind;
use spookery_kernel::api::visibility::{self, Seat, Seen, Told};
use spookery_kernel::api::Reason;

use crate::state::Round;

/// Deliver narrate lines and build each viewer's change summary.
pub(crate) fn run(round: &mut Round, draft: &mut Draft) -> Result<(), Reason> {
    round.steps_run.push("observe".to_string());
    let canon = draft.canon();
    let now = draft.now();
    let gm = visibility::observe(canon, now, &Seat::Gm, &round.changes, &round.effects);
    round.summary = gm.changes;
    let mut viewers: BTreeSet<String> = now.actors.keys().cloned().collect();
    for effect in &round.effects {
        if let EffectKind::Narrate { to, .. } = &effect.class {
            viewers.extend(to.iter().cloned());
        }
    }
    let mut lines = Vec::new();
    let mut seen = Vec::new();
    for id in viewers {
        let delivery = visibility::observe(
            canon,
            now,
            &Seat::Actor(id.clone()),
            &round.changes,
            &round.effects,
        );
        for text in delivery.lines {
            lines.push(Told {
                to: id.clone(),
                text,
            });
        }
        if !delivery.changes.is_empty() {
            seen.push(Seen {
                viewer: id,
                changes: delivery.changes,
                line: delivery.line,
            });
        }
    }
    round.lines = lines;
    round.seen = seen;
    Ok(())
}
