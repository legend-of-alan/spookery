//! Who controls each actor: a player or the engine.

use serde::{Deserialize, Serialize};

use crate::intent::Viewer;
use crate::state::component::{Component, Ref};
use crate::state::entity;

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

impl Ref<'_, entity::Actors, ControlType> {
    /// Who controls the actor.
    pub fn get(&self) -> Option<ControlType> {
        self.store().get(self.id).copied()
    }

    /// Whether a player controls the actor. An actor the save lacks is not a PC.
    pub fn is_pc(&self) -> bool {
        self.get() == Some(ControlType::Pc)
    }

    /// The seat the actor acts from. An actor the save lacks has none.
    pub fn viewer(&self) -> Option<Viewer> {
        self.get().map(|control| match control {
            ControlType::Pc => Viewer::Player(self.id.clone()),
            ControlType::Npc => Viewer::Npc(self.id.clone()),
        })
    }
}
