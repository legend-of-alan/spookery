//! Who controls each actor: a player or the engine.

use serde::{Deserialize, Serialize};

use crate::ecs::component::{Component, Ref};
use crate::ecs::entity;

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
    const ID: &'static str = "control_type";
}

impl Ref<'_, entity::Actors, ControlType> {
    /// Who controls the actor.
    pub fn get(&self) -> Option<ControlType> {
        self.store().get(self.id).copied()
    }

    /// Whether a player controls the actor. An actor the save lacks is not a PC.
    pub fn is_pc(&self) -> bool {
        self.get() == Some(ControlType::Pc)
    }
}
