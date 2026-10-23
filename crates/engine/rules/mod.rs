//! Conditions, costs, gates, rolls, and effects.

pub(crate) mod apply;
pub(crate) mod check;
pub(crate) mod cond;
pub(crate) mod cost;
pub(crate) mod effect;
pub(crate) mod expand;
pub(crate) mod expr;
pub(crate) mod gate;
pub(crate) mod habits;
pub(crate) mod perceive;
pub(crate) mod roll;
pub(crate) mod rule;

pub use gate::GateRefusal;
pub(crate) use roll::{faces_from, roll, RollInput};
pub use roll::{MoveReport, RollResult};

#[cfg(not(any(test, feature = "testing")))]
pub(crate) use roll::RollKey;
#[cfg(any(test, feature = "testing"))]
pub use roll::{faces, RollKey};
