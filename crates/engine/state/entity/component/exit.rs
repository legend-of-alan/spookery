//! Open exits: the doors that stand open, each on its place.

use crate::state::component::Component;

/// An exit whose door is open, on its place under the exit's name.
pub struct OpenExit;

impl Component for OpenExit {
    type Value = ();
    const NAME: &'static str = "exit_open";
}
