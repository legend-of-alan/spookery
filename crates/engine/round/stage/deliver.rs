//! Decide what the GM and each seated viewer may see and hear.

use std::collections::BTreeSet;

use crate::canon::Canon;
use crate::effect::{Effect, EffectKind};
use crate::key::Change;
use crate::model::{Presence, State};
use crate::visibility::{self, Recipient, RecipientLine, ViewerChanges};

/// What the GM and each viewer may see and hear.
pub(crate) struct Deliveries {
    pub summary: Vec<String>,
    pub lines: Vec<RecipientLine>,
    pub seen: Vec<ViewerChanges>,
}

/// Deliver narrate lines and build each viewer's change summary.
/// An empty table gets only the GM summary.
pub(crate) fn run(
    canon: &Canon,
    now: &State,
    presence: &Presence,
    changes: &[Change],
    effects: &[Effect],
) -> Deliveries {
    let gm = visibility::observe(canon, now, &Recipient::Gm, changes, effects);
    let mut delivered = Deliveries {
        summary: gm.changes,
        lines: Vec::new(),
        seen: Vec::new(),
    };
    if presence.players == 0 {
        return delivered;
    }
    let mut viewers: BTreeSet<String> = now.actors.keys().cloned().collect();
    for effect in effects {
        if let EffectKind::Narrate { to, .. } = &effect.kind {
            viewers.extend(to.iter().cloned());
        }
    }
    for id in viewers {
        let delivery =
            visibility::observe(canon, now, &Recipient::Actor(id.clone()), changes, effects);
        for text in delivery.narrations {
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
