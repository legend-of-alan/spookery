//! What a rule is read against.

use crate::canon::Canon;
use crate::ecs::id::PlaceId;
use crate::ecs::{self, component, Presence};
use crate::intent::Intent;
use crate::rules::expr::eval::LetValue;
use crate::Reason;

/// The canon, the save, who is seated, and the intent a rule's conditions, costs and effects read.
/// The canon has a lifetime of its own, so a rule found through it can outlive the borrow of the
/// save.
///
/// The bare name stands beside `std::task::Context`: the engine library has no async code, so
/// that one is never in scope here, and two host files are its only users in the workspace.
pub(crate) struct Context<'c, 'a> {
    pub canon: &'c Canon,
    pub world: &'a ecs::World,
    /// Who is seated, for the `players:` condition and NPC habits.
    pub presence: &'a Presence,
    pub intent: &'a Intent,
    /// The rule's `let:` values rolled so far, which its numbers read by id.
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

    /// The item a rule line's `id:` names: the intent's target token for `target`, or else
    /// `id` itself.
    pub(crate) fn item_id(&self, id: &str) -> Result<String, Reason> {
        if id == "target" {
            let token = self.intent.target.token();
            return token.map(str::to_string).ok_or(Reason::Schema);
        }
        Ok(id.to_string())
    }

    /// The place the acting actor stands in, which `here` names. A walker is in no place, so it
    /// is `UNKNOWN_ID`.
    pub(crate) fn actor_place_id(&self) -> Result<&'a str, Reason> {
        let actor_id = self.intent.actor_id.as_ref().ok_or(Reason::Schema)?;
        let place = self.world.actors().get::<component::Place>(actor_id);
        place
            .place_id()
            .map(PlaceId::as_str)
            .ok_or(Reason::UnknownId)
    }
}
