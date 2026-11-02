//! What a rule is read against.

use crate::canon::Canon;
use crate::intent::Intent;
use crate::model::{Presence, State};

/// The canon, the save, who is seated, and the intent a rule's conditions, costs and effects read.
///
/// The bare name stands beside `std::task::Context`: the engine library has no async code, so
/// that one is never in scope here, and two server files are its only users in the workspace.
pub(crate) struct Context<'a> {
    pub canon: &'a Canon,
    pub now: &'a State,
    /// Who is seated, for the `players:` condition and NPC habits.
    pub presence: &'a Presence,
    pub intent: &'a Intent,
}

impl<'a> Context<'a> {
    pub(crate) fn new(
        canon: &'a Canon,
        now: &'a State,
        presence: &'a Presence,
        intent: &'a Intent,
    ) -> Self {
        Self {
            canon,
            now,
            presence,
            intent,
        }
    }
}
