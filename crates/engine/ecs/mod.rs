//! The ECS: the save as the file holds it, and the `World` that owns its components and reads
//! them against the pack's canon.

pub(crate) mod change;
pub(crate) mod entity;
pub use entity::component;
pub(crate) mod id;
pub(crate) mod nested_map;
pub(crate) mod prelude;
mod transaction;
pub(crate) mod undo;

use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

#[cfg(any(test, feature = "testing"))]
pub use component::Act;
pub use component::PendingAction;
pub(crate) use component::WindowCount;
use id::ActorId;
#[cfg(test)]
pub(crate) use transaction::Savepoint;
use transaction::Transaction;

use crate::canon::Canon;

/// The save as the file holds it: the clock and each collection's components. It has no canon
/// and no writers; `World::new` opens it to read and write.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Save {
    pub schema: String,
    pub world_id: String,
    /// The world's dice seed. Empty in a shipped save; chosen the first time the world opens
    /// for play.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub seed: String,
    pub pack_rev: String,
    /// Engine rules version. Missing means 0, which is not the current version.
    #[serde(default)]
    pub rules_rev: u64,
    pub state_rev: u64,
    pub round_index: u64,
    /// Ticks since the world began.
    pub world_time: u64,
    pub last_round_at: String,
    actors: entity::Actors,
    #[serde(default)]
    places: entity::Places,
    #[serde(default)]
    stories: entity::Stories,
    #[serde(default)]
    globals: entity::Globals,
}

/// The ECS world, which owns every component, like Bevy's `World`: the save, the pack's canon it
/// is read against, and the round's transaction. A `World` never exists without its canon.
#[derive(Clone, Debug)]
pub struct World {
    save: Save,
    canon: Arc<Canon>,
    transaction: Transaction,
}

impl Deref for World {
    type Target = Save;
    fn deref(&self) -> &Save {
        &self.save
    }
}

impl DerefMut for World {
    fn deref_mut(&mut self) -> &mut Save {
        &mut self.save
    }
}

/// One collection with the pack's canon, borrowed to read. It is `Copy`, and reads through `get`,
/// `contains` and `query`; only `World`'s getters build one.
pub struct Reader<'a, E> {
    components: &'a E,
    canon: &'a Canon,
}

impl<E> Clone for Reader<'_, E> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<E> Copy for Reader<'_, E> {}

impl<'a, E> Reader<'a, E> {
    fn new(components: &'a E, canon: &'a Canon) -> Self {
        Reader { components, canon }
    }
}

impl<'a, E: entity::Collection> Reader<'a, E> {
    /// `id`'s component `C`, borrowed. A collection that does not hold `C` has no reads on it.
    pub fn get<C: component::Component>(self, id: &'a E::Id) -> component::Ref<'a, E, C> {
        component::Ref::new(self, id)
    }

    /// Whether `id` is one of this collection's entities.
    pub fn contains(self, id: &E::Id) -> bool {
        self.components.contains(self.canon, id)
    }

    /// What `D` reads from each entity that holds it: Bevy's `World::query`.
    pub fn query<D: entity::QueryData<E>>(self) -> entity::Query<'a, E, D> {
        self.query_filtered()
    }

    /// What `D` reads from each entity that holds it and passes `F`: Bevy's
    /// `World::query_filtered`.
    pub fn query_filtered<D, F>(self) -> entity::Query<'a, E, D, F>
    where
        D: entity::QueryData<E>,
        F: entity::QueryFilter<E>,
    {
        let data_and_filter = std::marker::PhantomData;
        entity::Query {
            entities: self,
            data_and_filter,
        }
    }
}

/// One collection's components, with the round's transaction and the pack's canon, borrowed
/// apart from the rest of the save: the only way to write them. It reads and writes only
/// through `get_mut` and its own methods, and only `World`'s `*_mut` getters build one.
///
/// ```compile_fail
/// # fn bypass(world: &mut spookery_engine::ecs::World) {
/// *world.actors_mut() = Default::default();
/// # }
/// ```
///
/// ```compile_fail
/// # fn bypass(world: &spookery_engine::ecs::World) {
/// let _ = world.actors().control_type.len();
/// # }
/// ```
pub struct Writer<'a, E> {
    components: &'a mut E,
    transaction: &'a mut Transaction,
    canon: &'a Canon,
}

impl<'a, E> Writer<'a, E> {
    /// The one place a `Writer` is built; only `World`'s `*_mut` getters call it.
    fn new(components: &'a mut E, transaction: &'a mut Transaction, canon: &'a Canon) -> Self {
        Writer {
            components,
            transaction,
            canon,
        }
    }
}

impl<E: entity::Collection> Writer<'_, E> {
    /// `id`'s component `C`, to write. A collection that does not hold `C` has no writes on it.
    pub fn get_mut<'a, C>(&'a mut self, id: &'a E::Id) -> component::Mut<'a, E, C>
    where
        C: component::Component,
    {
        let writer = Writer::new(&mut *self.components, &mut *self.transaction, self.canon);
        component::Mut::new(writer, id)
    }
}

impl World {
    /// The ECS world of `save`, read against `canon`.
    pub fn new(save: Save, canon: Arc<Canon>) -> World {
        World {
            save,
            canon,
            transaction: Transaction::default(),
        }
    }

    pub fn canon(&self) -> &Arc<Canon> {
        &self.canon
    }

    /// The actors in `place`, in id order.
    pub(crate) fn actors_at(&self, place: &str) -> Vec<ActorId> {
        let actors = self.actors().query::<(ActorId, &component::Place)>();
        let here = actors.iter().filter(|(_, at)| *at == place);
        here.map(|(id, _)| id.clone()).collect()
    }

    pub fn actors(&self) -> Reader<'_, entity::Actors> {
        Reader::new(&self.save.actors, &self.canon)
    }

    pub fn places(&self) -> Reader<'_, entity::Places> {
        Reader::new(&self.save.places, &self.canon)
    }

    pub fn stories(&self) -> Reader<'_, entity::Stories> {
        Reader::new(&self.save.stories, &self.canon)
    }

    pub fn globals(&self) -> Reader<'_, entity::Globals> {
        Reader::new(&self.save.globals, &self.canon)
    }

    pub fn actors_mut(&mut self) -> Writer<'_, entity::Actors> {
        Writer::new(&mut self.save.actors, &mut self.transaction, &self.canon)
    }

    pub fn places_mut(&mut self) -> Writer<'_, entity::Places> {
        Writer::new(&mut self.save.places, &mut self.transaction, &self.canon)
    }

    pub fn stories_mut(&mut self) -> Writer<'_, entity::Stories> {
        Writer::new(&mut self.save.stories, &mut self.transaction, &self.canon)
    }

    pub fn globals_mut(&mut self) -> Writer<'_, entity::Globals> {
        Writer::new(&mut self.save.globals, &mut self.transaction, &self.canon)
    }

    /// Every entity, each read and write going to the collection its `entity::Id` names.
    pub fn entities(&self) -> Reader<'_, Save> {
        Reader::new(&self.save, &self.canon)
    }

    pub fn entities_mut(&mut self) -> Writer<'_, Save> {
        Writer::new(&mut self.save, &mut self.transaction, &self.canon)
    }
}

/// Who is seated. The host's seats own it and hand it to each round; the save never holds it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Presence {
    pub players: u32,
    pub actors_present: Vec<ActorId>,
}

/// The place and exit names in an exit's id, `exit:<place>.<exit>`: where a walker stands
/// until they arrive. `None` for any other id.
pub(crate) fn exit_parts(id: &str) -> Option<(&str, &str)> {
    let (place, exit) = id.strip_prefix("exit:")?.split_once('.')?;
    (!place.is_empty() && !exit.is_empty()).then_some((place, exit))
}

pub(crate) fn sort_unique<T: Ord>(items: &mut Vec<T>) {
    items.sort();
    items.dedup();
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryLine {
    pub round_index: u64,
    pub actor_id: ActorId,
    pub intent_id: String,
    pub fact_ids: Vec<String>,
    pub tags: Vec<String>,
    pub text: Option<String>,
}
