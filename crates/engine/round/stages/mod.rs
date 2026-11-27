//! One round, in the order `round::stage` runs them.

pub(crate) mod advance;
pub(crate) mod apply;
pub(crate) mod deliver;
pub(crate) mod end;
pub(crate) mod pick;
pub(crate) mod record;
pub(crate) mod triggers;

use crate::ecs;
use crate::round::{Ledger, Time};

/// What the stages leave for `record`: the round's working copy of the save, its time, its ledger,
/// and what each seat hears.
pub(crate) struct Output {
    pub(crate) world: ecs::World,
    pub(crate) clock: Time,
    pub(crate) ledger: Ledger,
    pub(crate) delivered: deliver::Deliveries,
}
