//! A store: a value, and beside it the undo of each change made to it since the last accept,
//! so a round can roll it back; the changes the save's maps and lists take; and the map of
//! each entity's components that a kind's store holds.

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::ops::Deref;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A typed change to a `T`.
pub trait Edit<T>: Sized {
    /// Make this change to `value` and give back the edit that undoes it. `None` when `value`
    /// already was so, and then `value` is left as it was.
    fn apply(self, value: &mut T) -> Option<Self>;
}

/// A value, and the undo of each change made to it since the last accept, oldest first, each
/// with the seq the ledger issued for that change. In a round only a change moves the value.
/// The save on disk holds the value alone.
#[derive(Debug, Serialize, Deserialize)]
#[serde(
    transparent,
    bound(serialize = "T: Serialize", deserialize = "T: Deserialize<'de>")
)]
pub struct Store<T, U> {
    value: T,
    #[serde(skip)]
    log: Vec<(u64, U)>,
}

/// One kind of component on every entity of one kind, such as every actor's meters.
pub type EntityStore<V, E> = Store<EntityMaps<V>, EntityEdit<V, E>>;

/// Values by key, such as the world's meters or every actor's record.
pub type KeyStore<V, E> = Store<BTreeMap<String, V>, MapEdit<V, E>>;

/// A copy holds the value alone: the undo stays with the original.
impl<T: Clone, U> Clone for Store<T, U> {
    fn clone(&self) -> Self {
        Self::from(self.value.clone())
    }
}

impl<T: Default, U> Default for Store<T, U> {
    fn default() -> Self {
        Self::from(T::default())
    }
}

impl<T, U> From<T> for Store<T, U> {
    fn from(value: T) -> Self {
        Self {
            value,
            log: Vec::new(),
        }
    }
}

/// Two stores are equal when they hold equal values, whatever changes led there.
impl<T: PartialEq, U> PartialEq for Store<T, U> {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl<T: Eq, U> Eq for Store<T, U> {}

impl<T, U> Deref for Store<T, U> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.value
    }
}

impl<T, U> Store<T, U> {
    /// Whether a change since the last accept can still be undone.
    pub(super) fn has_undo(&self) -> bool {
        !self.log.is_empty()
    }

    /// Forget every undo: nothing can roll the value back past here.
    pub(super) fn clear_log(&mut self) {
        self.log.clear();
    }

    /// The value, to change in place past the log: outside a round, where nothing can be
    /// rolled back. A change here while a change can still be undone would be lost to a
    /// rollback, so that panics.
    pub(super) fn frontier_mut(&mut self) -> &mut T {
        assert!(!self.has_undo(), "a store changed past its log");
        &mut self.value
    }
}

impl<T, U: Edit<T>> Store<T, U> {
    /// Make `edit`, and when it moved the value keep its undo with the seq `seq` issues then.
    /// Whether it moved.
    pub(super) fn apply(&mut self, edit: U, seq: impl FnOnce() -> u64) -> bool {
        let Some(undo) = edit.apply(&mut self.value) else {
            return false;
        };
        self.log.push((seq(), undo));
        true
    }

    /// Undo each change whose seq is `seq` or later, newest first.
    pub(super) fn undo_to(&mut self, seq: u64) {
        while self.log.last().is_some_and(|(at, _)| *at >= seq) {
            let (_, undo) = self.log.pop().expect("an undo");
            // The change it undoes was the newest one, so the value is as that change left it.
            let redo = undo.apply(&mut self.value);
            debug_assert!(redo.is_some(), "an undo changed nothing");
        }
    }
}

/// A change to the entry under a key in a map: its value edited, or the entry put on,
/// replaced or taken off. Making it gives back its undo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MapEdit<V, E> {
    /// Edit the value under this key.
    Value(String, E),
    Entry(Entry<V>),
}

impl<V: PartialEq, E: Edit<V>> Edit<BTreeMap<String, V>> for MapEdit<V, E> {
    fn apply(self, map: &mut BTreeMap<String, V>) -> Option<Self> {
        match self {
            MapEdit::Value(key, edit) => {
                let undo = edit.apply(map.get_mut(&key)?)?;
                Some(MapEdit::Value(key, undo))
            }
            MapEdit::Entry(entry) => entry.apply(map).map(MapEdit::Entry),
        }
    }
}

/// A change to entity `id`'s map in a kind's store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntityEdit<V, E> {
    id: String,
    edit: MapEdit<V, E>,
}

/// An entity's map is put back when a change puts on its first entry, and dropped when one
/// takes off its last, so the store reads the same after an add is rolled back.
impl<V: PartialEq, E: Edit<V>> Edit<EntityMaps<V>> for EntityEdit<V, E> {
    fn apply(self, maps: &mut EntityMaps<V>) -> Option<Self> {
        let EntityEdit { id, edit } = self;
        let undo = match maps.0.get_mut(&id) {
            Some(parts) => {
                let undo = edit.apply(parts);
                if parts.is_empty() {
                    maps.0.remove(&id);
                }
                undo
            }
            None => {
                let mut parts = BTreeMap::new();
                let undo = edit.apply(&mut parts);
                if !parts.is_empty() {
                    maps.0.insert(id.clone(), parts);
                }
                undo
            }
        };
        Some(EntityEdit { id, edit: undo? })
    }
}

/// A component with no value, such as a flag, takes no edit: it is only put on or taken off.
impl<T> Edit<T> for Infallible {
    fn apply(self, _: &mut T) -> Option<Self> {
        match self {}
    }
}

/// Each entity's own map of one kind of component, by its id. An entity with none has no
/// entry. The save writes it as each entity's id with its components by key:
/// `pc_rowan: {grit: {base: 1, offset: 0}}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct EntityMaps<V>(BTreeMap<String, BTreeMap<String, V>>);

impl<V> Default for EntityMaps<V> {
    fn default() -> Self {
        EntityMaps(BTreeMap::new())
    }
}

/// An entity listed with no components has no entry.
impl<'de, V: Deserialize<'de>> Deserialize<'de> for EntityMaps<V> {
    fn deserialize<D: Deserializer<'de>>(from: D) -> Result<Self, D::Error> {
        let mut on = BTreeMap::<String, BTreeMap<String, V>>::deserialize(from)?;
        on.retain(|_, parts| !parts.is_empty());
        Ok(EntityMaps(on))
    }
}

impl<V> EntityMaps<V> {
    /// The components on entity `id`, by key, when it holds any.
    pub fn of(&self, id: &str) -> Option<&BTreeMap<String, V>> {
        self.0.get(id)
    }

    /// The component on entity `id` under `key`.
    pub fn get(&self, id: &str, key: &str) -> Option<&V> {
        self.0.get(id)?.get(key)
    }

    /// Every component, in entity and then key order.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &String, &V)> {
        let entities = self.0.iter();
        entities.flat_map(|(id, parts)| parts.iter().map(move |(key, part)| (id, key, part)))
    }
}

/// One entity's components of one kind, to read: its own map, or none when it holds none.
pub struct Parts<'a, V>(pub(super) Option<&'a BTreeMap<String, V>>);

impl<'a, V> Parts<'a, V> {
    /// The component under `key`.
    pub fn get(&self, key: &str) -> Option<&'a V> {
        self.0?.get(key)
    }

    /// Every component, in key order.
    pub fn iter(&self) -> impl Iterator<Item = (&'a String, &'a V)> {
        self.0.into_iter().flatten()
    }
}

/// One entity's components of one kind, to write: its entry in its kind's store, by its id,
/// or the world's own store.
pub(super) enum PartsMut<'a, V, E> {
    Stored(&'a mut EntityStore<V, E>, &'a str),
    World(&'a mut KeyStore<V, E>),
}

impl<V: PartialEq, E: Edit<V>> PartsMut<'_, V, E> {
    /// Make `edit` on this entity's components, as `Store::apply` says.
    pub(super) fn apply(self, edit: MapEdit<V, E>, seq: impl FnOnce() -> u64) -> bool {
        match self {
            PartsMut::Stored(store, id) => {
                let id = id.to_string();
                store.apply(EntityEdit { id, edit }, seq)
            }
            PartsMut::World(store) => store.apply(edit, seq),
        }
    }
}

/// Marks with no value, such as flags, written as the list of their keys: `[calm, wary]`.
pub(crate) mod marks {
    use super::*;

    pub(crate) fn serialize<S: Serializer>(
        marks: &KeyStore<(), Infallible>,
        out: S,
    ) -> Result<S::Ok, S::Error> {
        out.collect_seq(marks.keys())
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
        from: D,
    ) -> Result<KeyStore<(), Infallible>, D::Error> {
        let keys = Vec::<String>::deserialize(from)?;
        let marks: BTreeMap<_, _> = keys.into_iter().map(|key| (key, ())).collect();
        Ok(Store::from(marks))
    }

    /// A store of marks, written as each entity's id with the list of its keys:
    /// `mira: [calm, wary]`.
    pub(crate) mod by_entity {
        use super::super::*;

        pub(crate) fn serialize<S: Serializer>(
            store: &EntityStore<(), Infallible>,
            out: S,
        ) -> Result<S::Ok, S::Error> {
            let entities = store.0.iter();
            out.collect_map(entities.map(|(id, marks)| (id, marks.keys().collect::<Vec<_>>())))
        }

        pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
            from: D,
        ) -> Result<EntityStore<(), Infallible>, D::Error> {
            let mut on = BTreeMap::new();
            for (id, keys) in BTreeMap::<String, Vec<String>>::deserialize(from)? {
                let marks: BTreeMap<_, _> = keys.into_iter().map(|key| (key, ())).collect();
                if !marks.is_empty() {
                    on.insert(id, marks);
                }
            }
            Ok(Store::from(EntityMaps(on)))
        }
    }
}

/// Map entry `id` takes `value`, `None` being no entry. Its undo holds the entry it replaced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry<V> {
    pub id: String,
    pub value: Option<V>,
}

impl<V: PartialEq> Edit<BTreeMap<String, V>> for Entry<V> {
    fn apply(self, map: &mut BTreeMap<String, V>) -> Option<Self> {
        if map.get(&self.id) == self.value.as_ref() {
            return None;
        }
        let old = match self.value {
            Some(new) => map.insert(self.id.clone(), new),
            None => map.remove(&self.id),
        };
        Some(Entry {
            id: self.id,
            value: old,
        })
    }
}

/// Swap `value` into `field` when it differs, and give back the value it replaced.
pub(crate) fn swap<T: PartialEq>(field: &mut T, value: T) -> Option<T> {
    (*field != value).then(|| std::mem::replace(field, value))
}
