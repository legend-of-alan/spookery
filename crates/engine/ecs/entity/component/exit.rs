//! Open exits, each on its place.

use crate::ecs::component::Component;

/// An open exit, on its place under the exit's id.
pub struct OpenExit; // repeats module: mirrors the save key places.open_exits

impl Component for OpenExit {
    type Value = ();
    const ID: &'static str = "exit_open";
}
