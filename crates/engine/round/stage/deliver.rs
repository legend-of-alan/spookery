//! Decide what the GM and each seated viewer may see and hear.

use crate::canon::Canon;
use crate::rules::effect::Effect;
use crate::rules::visibility::{self, Recipient, RecipientLine, ViewerChanges};
use crate::state::change::Change;
use crate::state::{Presence, State};

/// What the GM and each viewer may see and hear. Passed from the deliver stage to `record`,
/// which puts it on the `RoundOutput`.
pub(crate) struct Deliveries {
    pub changed_keys: Vec<String>,
    pub lines: Vec<RecipientLine>,
    pub seen: Vec<ViewerChanges>,
}

/// Deliver narrate lines and build each viewer's change summary.
/// An empty table gets only the GM summary.
pub(crate) fn run(
    canon: &Canon,
    state: &State,
    presence: &Presence,
    changes: &[Change],
    effects: &[Effect],
) -> Deliveries {
    let gm = visibility::show_to(canon, state, &Recipient::Gm, changes, effects);
    let mut delivered = Deliveries {
        changed_keys: gm.changes,
        lines: Vec::new(),
        seen: Vec::new(),
    };
    if presence.players == 0 {
        return delivered;
    }
    for id in state.actors().control_type.keys().cloned() {
        let delivery = visibility::show_to(
            canon,
            state,
            &Recipient::Actor(id.clone()),
            changes,
            effects,
        );
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
