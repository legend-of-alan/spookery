//! The actors' components, one store per component, keyed by actor id.

#[cfg(any(test, feature = "testing"))]
mod fixtures;

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::canon::Canon;
use crate::ecs::component::modifier::Modifier;
use crate::ecs::component::{self, Act, Meter};
use crate::ecs::id::ActorId;
use crate::ecs::{entity, nested_map, undo, Reader};

/// Every actor's components, by actor id. An actor exists when it has a control type;
/// `control_type`, `place` and `acts` have an entry for every actor, the rest only for actors
/// holding something.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Actors {
    /// Who controls each actor, a player or the engine. An actor exists when it has one.
    control_type: undo::Store<BTreeMap<ActorId, component::ControlType>>,
    /// Where each actor stands: a place id, or `exit:<place>.<exit>` while they walk.
    place: undo::DenseStore<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    wielded: undo::Store<BTreeMap<ActorId, String>>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    known_facts: undo::Store<BTreeMap<ActorId, BTreeSet<String>>>,
    /// Each place an actor has stood at the end of a round, with the names of the exits they
    /// saw there then.
    #[serde(with = "nested_map", skip_serializing_if = "BTreeMap::is_empty")]
    visited: undo::Store<component::Map<ActorId, component::Visit>>,
    acts: undo::DenseStore<Act>,
    #[serde(with = "nested_map")]
    items: undo::Store<component::Map<ActorId, component::Item>>,
    #[serde(with = "nested_map")]
    meters: undo::Store<component::Map<ActorId, Meter>>,
    /// The modifiers each actor holds, by modifier id.
    #[serde(with = "nested_map")]
    modifiers: undo::Store<component::Map<ActorId, Modifier>>,
    #[serde(with = "nested_map::lists")]
    flags: undo::Store<component::Map<ActorId, component::Flag>>,
}

impl entity::Collection for Actors {
    type Id = ActorId;

    fn contains(&self, _: &Canon, id: &ActorId) -> bool {
        self.control_type.contains_key(id)
    }

    fn ids<'a>(&'a self, _: &'a Canon) -> entity::query::Ids<'a, Self> {
        Box::new(self.control_type.keys())
    }
}

entity::has!(Actors, component::Item, items);
entity::has!(Actors, Meter, meters);
entity::has!(Actors, component::Flag, flags);
entity::has!(Actors, Modifier, modifiers);
entity::has!(Actors, component::Visit, visited);
entity::has!(Actors, component::ControlType, control_type: undo::Store<BTreeMap<ActorId, component::ControlType>>);
entity::has!(Actors, component::Place, place: undo::DenseStore<String>);
entity::has!(Actors, component::WieldedItem, wielded: undo::Store<BTreeMap<ActorId, String>>);
entity::has!(Actors, component::KnownFacts, known_facts: undo::Store<BTreeMap<ActorId, BTreeSet<String>>>);
entity::has!(Actors, Act, acts: undo::DenseStore<Act>);

impl Reader<'_, Actors> {
    /// Each store's key, as the save writes it, with each actor it carries something for. The
    /// control type is left out: an actor exists when it has one.
    pub(crate) fn carriers_by_store(&self) -> BTreeSet<(&'static str, &ActorId)> {
        // Every store, with no `..`: a store added later must be listed here to compile.
        let Actors {
            control_type: _,
            place,
            wielded,
            known_facts,
            visited,
            acts,
            items,
            meters,
            modifiers,
            flags,
        } = self.components;
        (place.keys().map(|id| ("place", id)))
            .chain(wielded.keys().map(|id| ("wielded", id)))
            .chain(known_facts.keys().map(|id| ("known_facts", id)))
            .chain(visited.keys().map(|(id, _)| ("visited", id)))
            .chain(acts.keys().map(|id| ("acts", id)))
            .chain(items.keys().map(|(id, _)| ("items", id)))
            .chain(meters.keys().map(|(id, _)| ("meters", id)))
            .chain(modifiers.keys().map(|(id, _)| ("modifiers", id)))
            .chain(flags.keys().map(|(id, _)| ("flags", id)))
            .collect()
    }
}

impl Actors {
    /// Every store, as the history a rollback undoes and an accept clears.
    pub(in crate::ecs) fn history_mut(&mut self) -> [&mut dyn undo::History; 10] {
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
