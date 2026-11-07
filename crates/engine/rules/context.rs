//! What a rule is read against.

use crate::canon::Canon;
use crate::intent::Intent;
use crate::model::{Presence, State};

/// The canon, the save, who is seated, and the intent a rule's conditions, costs and effects read.
///
/// The bare name stands beside `std::task::Context`: the engine library has no async code, so
/// that one is never in scope here, and two server files are its only users in the workspace.
/// The canon has its own lifetime `'c`, so a rule or exit read from it can outlive the borrow of
/// the save: resolve picks rules against the save that apply then writes.
pub(crate) struct Context<'c, 'a> {
    pub canon: &'c Canon,
    pub state: &'a State,
    /// Who is seated, for the `players:` condition and NPC habits.
    pub presence: &'a Presence,
    pub intent: &'a Intent,
}

impl<'c, 'a> Context<'c, 'a> {
    pub(crate) fn new(
        canon: &'c Canon,
        state: &'a State,
        presence: &'a Presence,
        intent: &'a Intent,
    ) -> Self {
        Self {
            canon,
            state,
            presence,
            intent,
        }
    }
}
