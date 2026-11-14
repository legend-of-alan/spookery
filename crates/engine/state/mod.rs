//! The save as the file holds it, and the live save that reads it against the pack's canon.

pub(crate) mod change;
pub mod component;
pub(crate) mod entity;
#[cfg(any(test, feature = "testing"))]
mod fixtures;
pub(crate) mod id;
pub(crate) mod nested_map;
mod notes;
pub(crate) mod prelude;
mod transaction;
pub mod undo;

use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

#[cfg(any(test, feature = "testing"))]
pub use component::Act;
pub(crate) use component::ActCounter;
pub use component::PendingAction;
use id::ActorId;
pub use notes::Note;
#[cfg(test)]
pub(crate) use transaction::Savepoint;
use transaction::Transaction;

use crate::canon::Canon;
use crate::intent::Viewer;

/// The save as the file holds it: the clock, each collection's components, and the notes. It has
/// no canon and no writers; `State::new` opens it to read and write.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Save {
    pub schema: String,
    pub world_id: String,
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
    world: entity::Globals,
    notes: undo::Store<Vec<Note>>,
}

/// The live save: what the file holds, the pack's canon it is read against, and the round's
/// transaction. A `State` never exists without its canon.
#[derive(Clone, Debug)]
pub struct State {
    save: Save,
    canon: Arc<Canon>,
    transaction: Transaction,
}

impl Deref for State {
    type Target = Save;
    fn deref(&self) -> &Save {
        &self.save
    }
}

impl DerefMut for State {
    fn deref_mut(&mut self) -> &mut Save {
        &mut self.save
    }
}

/// One collection with the pack's canon, borrowed to read. It is `Copy`, and reads the stores
/// through `Deref`; only `State`'s getters build one.
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

impl<'a, E: entity::Entities> Reader<'a, E> {
    /// `id`'s component `C`, borrowed. A collection that does not hold `C` has no reads on it.
    pub fn get<C: component::Component>(self, id: &'a E::Id) -> component::Ref<'a, E, C> {
        component::Ref::new(self, id)
    }

    /// Whether `id` is one of this collection's entities.
    pub fn contains(self, id: &E::Id) -> bool {
        self.components.contains(self.canon, id)
    }
}

impl<E> Deref for Reader<'_, E> {
    type Target = E;
    fn deref(&self) -> &E {
        self.components
    }
}

/// One collection's components, or the notes, with the round's transaction and the pack's canon,
/// borrowed apart from the rest of the save: the only way to write them. Reads go through `Deref`; there is no
/// `DerefMut`, and only `State`'s `*_mut` getters build one.
///
/// ```compile_fail
/// # fn bypass(state: &mut spookery_engine::State) {
/// *state.actors_mut() = Default::default();
/// # }
/// ```
///
/// ```compile_fail
/// # fn bypass(state: &mut spookery_engine::State) {
/// let control_type = *state.actors().control_type.values().next().unwrap();
/// state.actors().control_type.insert("x".into(), control_type);
/// # }
/// ```
pub struct Writer<'a, E> {
    components: &'a mut E,
    transaction: &'a mut Transaction,
    canon: &'a Canon,
}

impl<'a, E> Writer<'a, E> {
    /// The one place a `Writer` is built; only `State`'s `*_mut` getters call it.
    fn new(components: &'a mut E, transaction: &'a mut Transaction, canon: &'a Canon) -> Self {
        Writer {
            components,
            transaction,
            canon,
        }
    }
}

impl<E: entity::Entities> Writer<'_, E> {
    /// `id`'s component `C`, to write. A collection that does not hold `C` has no writes on it.
    pub fn get_mut<'a, C>(&'a mut self, id: &'a E::Id) -> component::Mut<'a, E, C>
    where
        C: component::Component,
    {
        let writer = Writer::new(&mut *self.components, &mut *self.transaction, self.canon);
        component::Mut::new(writer, id)
    }
}

impl<E> Deref for Writer<'_, E> {
    type Target = E;
    fn deref(&self) -> &E {
        self.components
    }
}

impl Save {
    pub fn actors(&self) -> &entity::Actors {
        &self.actors
    }

    pub fn places(&self) -> &entity::Places {
        &self.places
    }

    pub fn stories(&self) -> &entity::Stories {
        &self.stories
    }

    pub fn globals(&self) -> &entity::Globals {
        &self.world
    }

    pub fn notes(&self) -> &[Note] {
        &self.notes
    }

    /// The seat actor `id` acts from.
    pub(crate) fn viewer_of(&self, id: &ActorId) -> Option<Viewer> {
        (self.actors.control_type.get(id)).map(|control| control.viewer(id))
    }

    /// The actors in `place`, in id order.
    pub(crate) fn actors_at(&self, place: &str) -> Vec<ActorId> {
        self.actors
            .place
            .iter()
            .filter(|(_, at)| *at == place)
            .map(|(id, _)| id.clone())
            .collect()
    }
}

impl State {
    /// The live save of `save`, read against `canon`.
    pub fn new(save: Save, canon: Arc<Canon>) -> State {
        State {
            save,
            canon,
            transaction: Transaction::default(),
        }
    }

    pub fn canon(&self) -> &Arc<Canon> {
        &self.canon
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
        Reader::new(&self.save.world, &self.canon)
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
        Writer::new(&mut self.save.world, &mut self.transaction, &self.canon)
    }

    /// Every entity, each read and write going to the collection its `entity::Id` names.
    pub fn entities(&self) -> Reader<'_, Save> {
        Reader::new(&self.save, &self.canon)
    }

    pub fn entities_mut(&mut self) -> Writer<'_, Save> {
        Writer::new(&mut self.save, &mut self.transaction, &self.canon)
    }

    pub fn notes_mut(&mut self) -> Writer<'_, undo::Store<Vec<Note>>> {
        Writer::new(&mut self.save.notes, &mut self.transaction, &self.canon)
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
