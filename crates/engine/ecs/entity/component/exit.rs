//! Open exits, each on its place.

use crate::ecs::component::Component;

/// An open exit, on its place under the exit's id.
pub struct OpenExit; // repeats module: the component that marks an exit open

impl Component for OpenExit {
    type Value = ();
    const ID: &'static str = "exit_open";
}
