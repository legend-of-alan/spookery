//! Open exits: the doors that stand open, each on its place.

use crate::state::change;
use crate::state::component::{Component, Mut};
use crate::state::entity;
use crate::Reason;

/// An exit whose door is open, on its place under the exit's name.
pub struct OpenExit;

impl Component for OpenExit {
    type Value = ();
}

impl Mut<'_, entity::Places, OpenExit> {
    /// Open the door of `exit`, or shut it.
    pub fn set(&mut self, exit: &str, open: bool) -> Result<(), Reason> {
        let key = change::Key::ExitOpen(self.id.to_string(), exit.to_string());
        self.set_with_key(exit, open, key)
    }
}
