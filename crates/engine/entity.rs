//! Any entity's id, as rules name it. `State::entities` and `State::entities_mut` read and write
//! a component on the entity an `entity::Id` names, through `component::Ref` and
//! `component::Mut` as on a typed collection.

pub use crate::state::entity::{Id, IdRef};
