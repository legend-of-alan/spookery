//! Who controls each actor: a player or the engine.

use serde::{Deserialize, Serialize};

use crate::intent::Viewer;
use crate::state::component::{Component, Ref};
use crate::state::entity;
use crate::state::id::ActorId;

/// Who controls an actor: a player, for a PC, or the engine, for an NPC. An actor exists when it
/// has one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlType {
    Pc,
    Npc,
}

impl Component for ControlType {
    type Value = ControlType;
}

impl ControlType {
    /// The seat actor `id` with this control type acts from.
    pub fn viewer(self, id: &ActorId) -> Viewer {
        match self {
            ControlType::Pc => Viewer::Player(id.clone()),
            ControlType::Npc => Viewer::Npc(id.clone()),
        }
    }
}

impl Ref<'_, entity::Actors, ControlType> {
    /// Who controls the actor.
    pub fn get(&self) -> Option<ControlType> {
        self.entities.components.control_type.get(self.id).copied()
    }
}
