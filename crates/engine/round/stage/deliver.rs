//! Decide what the GM and each seated viewer may see and hear.

use crate::canon::Canon;
use crate::ecs::change::Change;
use crate::ecs::id::ActorId;
use crate::ecs::{self, Presence};
use crate::rules::effect::Effect;
use crate::rules::visibility::{self, Recipient, RecipientLine, ViewerChanges};

/// What the GM and each viewer may see and hear. Passed from the deliver stage to `record`,
/// which puts it on the `round::Output`.
pub(crate) struct Deliveries {
    pub changed_keys: Vec<String>,
    pub lines: Vec<RecipientLine>,
    pub seen: Vec<ViewerChanges>,
}

/// Deliver narrate lines and build each viewer's change summary.
/// An empty table gets only the GM summary.
pub(crate) fn run(
    canon: &Canon,
    world: &ecs::World,
    presence: &Presence,
    changes: &[Change],
    effects: &[Effect],
) -> Deliveries {
    let gm = visibility::show_to(canon, world, &Recipient::Gm, changes, effects);
    let mut delivered = Deliveries {
        changed_keys: gm.changed_keys,
        lines: Vec::new(),
        seen: Vec::new(),
    };
    if presence.players == 0 {
        return delivered;
    }
    for id in world.actors().query::<ActorId>().iter().cloned() {
        let delivery = visibility::show_to(
            canon,
            world,
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
        if !delivery.changed_keys.is_empty() {
            delivered.seen.push(ViewerChanges {
                viewer: id,
                changed_keys: delivery.changed_keys,
                changes: delivery.changes,
            });
        }
    }
    delivered
}
