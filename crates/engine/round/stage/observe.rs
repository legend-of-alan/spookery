//! Deliver narrate lines and build the change summary for each viewer.

use std::collections::BTreeSet;

use crate::canon::Canon;
use crate::effect::{Effect, EffectKind};
use crate::key::ChangeRecord;
use crate::model::Now;
use crate::visibility::{self, RecipientLine, Seat, ViewerChanges};

/// What the GM and each viewer may see and hear.
pub(crate) struct Delivered {
    pub summary: Vec<String>,
    pub lines: Vec<RecipientLine>,
    pub seen: Vec<ViewerChanges>,
}

/// Deliver narrate lines and build each viewer's change summary.
/// An empty table gets only the GM summary.
pub(crate) fn run(
    canon: &Canon,
    now: &Now,
    changes: &[ChangeRecord],
    effects: &[Effect],
) -> Delivered {
    let gm = visibility::observe(canon, now, &Seat::Gm, changes, effects);
    let mut delivered = Delivered {
        summary: gm.changes,
        lines: Vec::new(),
        seen: Vec::new(),
    };
    if now.presence.humans == 0 {
        return delivered;
    }
    let mut viewers: BTreeSet<String> = now.actors.keys().cloned().collect();
    for effect in effects {
        if let EffectKind::Narrate { to, .. } = &effect.class {
            viewers.extend(to.iter().cloned());
        }
    }
    for id in viewers {
        let delivery = visibility::observe(canon, now, &Seat::Actor(id.clone()), changes, effects);
        for text in delivery.lines {
            delivered.lines.push(RecipientLine {
                to: id.clone(),
                text,
            });
        }
        if !delivery.changes.is_empty() {
            delivered.seen.push(ViewerChanges {
                viewer: id,
                changes: delivery.changes,
                line: delivery.line,
            });
        }
    }
    delivered
}
