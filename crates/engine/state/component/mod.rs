//! What an entity holds, one module per component, with the rule each one's next value follows.

pub(in crate::state) mod flag;
pub(in crate::state) mod item;
pub(crate) mod meter;
pub(crate) mod modifier;
