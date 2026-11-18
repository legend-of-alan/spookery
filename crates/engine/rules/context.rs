//! What a rule is read against.

use crate::canon::Canon;
use crate::ecs::{self, Presence};
use crate::intent::{Intent, Source, Target, Viewer};
use crate::rules::roll::LetValue;

/// The canon, the save, who is seated, and the intent a rule's conditions, costs and effects read.
/// The canon has a lifetime of its own, so a rule found through it can outlive the borrow of the
/// save.
///
/// The bare name stands beside `std::task::Context`: the engine library has no async code, so
/// that one is never in scope here, and two server files are its only users in the workspace.
pub(crate) struct Context<'c, 'a> {
    pub canon: &'c Canon,
    pub world: &'a ecs::World,
    /// Who is seated, for the `players:` condition and NPC habits.
    pub presence: &'a Presence,
    pub intent: &'a Intent,
    /// The rule's `let:` values rolled so far, which its numbers read by name.
    pub lets: &'a [LetValue],
}

impl<'c, 'a> Context<'c, 'a> {
    pub(crate) fn new(
        canon: &'c Canon,
        world: &'a ecs::World,
        presence: &'a Presence,
        intent: &'a Intent,
    ) -> Self {
        Self {
            canon,
            world,
            presence,
            intent,
            lets: &[],
        }
    }

    /// This context with `lets` as the `let:` values its numbers read.
    pub(crate) fn bind<'b>(&'b self, lets: &'b [LetValue]) -> Context<'c, 'b> {
        Context { lets, ..*self }
    }
}

/// `viewer` doing `verb` to `target`, as an intent with no id: what `help`, exit locks, an NPC's
/// habit list and an exit's ticks in a view read a rule against, having no intent of their own.
pub(crate) fn probe(world: &ecs::World, viewer: &Viewer, verb: &str, target: Target) -> Intent {
    let (round, actor) = (world.round_index, Some(viewer.clone()));
    Intent::new(String::new(), round, actor, Source::Player, verb, target)
}
