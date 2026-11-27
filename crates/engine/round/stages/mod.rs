//! One round, in the order `round::stage` runs them.

pub(crate) mod advance;
pub(crate) mod apply;
pub(crate) mod deliver;
pub(crate) mod end;
pub(crate) mod pick;
pub(crate) mod record;
pub(crate) mod triggers;
