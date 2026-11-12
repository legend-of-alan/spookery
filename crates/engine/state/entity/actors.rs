//! The actors' components, one store per component, keyed by actor id.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::canon::Canon;
use crate::state::component::modifier::{self, Modifier};
use crate::state::component::{self, Act, Meter};
use crate::state::id::ActorId;
use crate::state::{entity, nested_map, undo};
use crate::Reason;

/// Every actor's components, by actor id. An actor exists when it has a control type;
/// `control_type`, `place` and `acts` have an entry for every actor, the rest only for actors
/// holding something.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Actors {
    /// Who controls each actor, a player or the engine. An actor exists when it has one.
    pub control_type: undo::Store<BTreeMap<ActorId, component::ControlType>>,
    /// Where each actor stands: a place id, or `exit:<place>.<exit>` while they walk.
    pub place: undo::DenseStore<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub wielded: undo::Store<BTreeMap<ActorId, String>>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub known_facts: undo::Store<BTreeMap<ActorId, Vec<String>>>,
    /// Each place an actor has stood at the end of a round, with the names of the exits they
    /// saw there then.
    #[serde(with = "nested_map", skip_serializing_if = "BTreeMap::is_empty")]
    pub visited: undo::Store<component::Map<ActorId, component::Visit>>,
    pub acts: undo::DenseStore<Act>,
    #[serde(with = "nested_map")]
    pub items: undo::Store<component::Map<ActorId, component::Item>>,
    #[serde(with = "nested_map")]
    pub meters: undo::Store<component::Map<ActorId, Meter>>,
    /// The modifiers each actor holds, by modifier id.
    #[serde(with = "nested_map")]
    pub modifiers: undo::Store<component::Map<ActorId, Modifier>>,
    #[serde(with = "nested_map::lists")]
    pub flags: undo::Store<component::Map<ActorId, component::Flag>>,
}

impl entity::Entities for Actors {
    type Id = ActorId;

    fn contains(&self, _: &Canon, id: &ActorId) -> bool {
        self.control_type.contains_key(id)
    }
}

entity::has!(Actors, component::Item, items);
entity::has!(Actors, Meter, meters);
entity::has!(Actors, component::Flag, flags);
entity::has!(Actors, Modifier, modifiers);
entity::has!(Actors, component::Visit, visited);

impl Actors {
    /// What `actor`'s modifiers add to meter `key`.
    pub fn modifier_sum(&self, actor: &ActorId, key: &str) -> Result<i64, Reason> {
        let held = nested_map::entries_of(&self.modifiers, actor).map(|(_, held)| held);
        modifier::sum(held, key)
    }

    /// Every store, as the log a rollback undoes and an accept clears.
    pub(in crate::state) fn logs_mut(&mut self) -> [&mut dyn undo::Log; 10] {
        // Every store, with no `..`: a store added later must be listed here to compile.
        let Actors {
            control_type,
            place,
            wielded,
            known_facts,
            visited,
            acts,
            items,
            meters,
            modifiers,
            flags,
        } = self;
        [
            control_type,
            place,
            wielded,
            known_facts,
            visited,
            acts,
            items,
            meters,
            modifiers,
            flags,
        ]
    }
}
