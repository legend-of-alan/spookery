//! A value that keeps the undo of each edit made to it since its frontier, so a round can roll
//! it back; the map that keeps such values by key; the store that keeps one such map per entity
//! of a kind; and the edits the save's lists and maps share.

use std::borrow::Borrow;
use std::collections::BTreeMap;
use std::fmt;
use std::ops::Deref;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A typed change to a `T`.
pub trait Edit<T>: Sized {
    /// Make this change to `value` and give back the edit that undoes it. `None` when `value`
    /// already was so, and then `value` is left as it was.
    fn apply(self, value: &mut T) -> Option<Self>;
}

/// A value, and the undo of each edit made to it since its frontier, oldest first. In a round
/// only an edit changes it. The save on disk holds the value alone.
#[derive(Serialize, Deserialize)]
#[serde(
    transparent,
    bound(serialize = "T: Serialize", deserialize = "T: Deserialize<'de>")
)]
pub struct Tracked<T, E> {
    value: T,
    #[serde(skip)]
    undo: Vec<E>,
}

/// A copy holds the value alone, at its frontier: the edits stay with the original.
impl<T: Clone, E> Clone for Tracked<T, E> {
    fn clone(&self) -> Self {
        Self::new(self.value.clone())
    }
}

impl<T: Default, E> Default for Tracked<T, E> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T, E> Tracked<T, E> {
    pub fn new(value: T) -> Self {
        Self {
            value,
            undo: Vec::new(),
        }
    }

    /// How many edits since the frontier can be undone.
    pub(super) fn edits(&self) -> usize {
        self.undo.len()
    }

    /// Keep the undo of only the newest `keep` edits. The value stays. `rebase(0)` makes the
    /// value the frontier.
    pub(super) fn rebase(&mut self, keep: usize) {
        let older = self.undo.len().saturating_sub(keep);
        self.undo.drain(..older);
    }

    /// The value, to change in place at its frontier: outside a round, where nothing can be
    /// rolled back. A change here while an edit can still be undone would be lost to a
    /// rollback, so that panics.
    pub(super) fn frontier_mut(&mut self) -> &mut T {
        assert!(
            self.undo.is_empty(),
            "a tracked value changed past its edits"
        );
        &mut self.value
    }
}

impl<T, E: Edit<T>> Tracked<T, E> {
    /// Make `edit` and keep its undo. Whether the value moved.
    pub(super) fn apply(&mut self, edit: E) -> bool {
        let Some(undo) = edit.apply(&mut self.value) else {
            return false;
        };
        self.undo.push(undo);
        true
    }

    /// Undo the newest edits, newest first, until `edits` are left.
    pub(super) fn rollback(&mut self, edits: usize) {
        for undo in self.undo.drain(edits..).rev() {
            // The edit it undoes was the newest one, so the value is as that edit left it.
            let redo = undo.apply(&mut self.value);
            debug_assert!(redo.is_some(), "an undo changed nothing");
        }
    }
}

/// A rollback or rebase of a tracked value, whatever it holds.
pub(super) trait Rewind {
    fn rollback(&mut self, edits: usize);
    fn rebase(&mut self, keep: usize);
}

impl<T, E: Edit<T>> Rewind for Tracked<T, E> {
    fn rollback(&mut self, edits: usize) {
        Tracked::rollback(self, edits);
    }

    fn rebase(&mut self, keep: usize) {
        Tracked::rebase(self, keep);
    }
}

impl<T, E> Deref for Tracked<T, E> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.value
    }
}

impl<T, E> From<T> for Tracked<T, E> {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

/// Two tracked values are equal when they hold equal values, whatever edits led there.
impl<T: PartialEq, E> PartialEq for Tracked<T, E> {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl<T: Eq, E> Eq for Tracked<T, E> {}

impl<T: fmt::Debug, E> fmt::Debug for Tracked<T, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.value.fmt(f)
    }
}

/// Tracked values by key, such as one entity's meters, the world's flags or every actor's
/// record. Reading gives values; only `State`'s writers put one on, take one off or edit one.
#[derive(Serialize, Deserialize)]
#[serde(
    transparent,
    bound(serialize = "V: Serialize", deserialize = "V: Deserialize<'de>")
)]
pub struct TrackedMap<V, E> {
    map: BTreeMap<String, Tracked<V, E>>,
}

impl<V, E> Default for TrackedMap<V, E> {
    fn default() -> Self {
        Self {
            map: BTreeMap::new(),
        }
    }
}

/// A copy holds the values alone, at their frontier, as `Tracked` copies do.
impl<V: Clone, E> Clone for TrackedMap<V, E> {
    fn clone(&self) -> Self {
        Self {
            map: self.map.clone(),
        }
    }
}

impl<V: PartialEq, E> PartialEq for TrackedMap<V, E> {
    fn eq(&self, other: &Self) -> bool {
        self.map == other.map
    }
}

impl<V: Eq, E> Eq for TrackedMap<V, E> {}

impl<V: fmt::Debug, E> fmt::Debug for TrackedMap<V, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.map.fmt(f)
    }
}

impl<V, E, Q: Ord + fmt::Debug + ?Sized> std::ops::Index<&Q> for TrackedMap<V, E>
where
    String: Borrow<Q>,
{
    type Output = V;

    fn index(&self, key: &Q) -> &V {
        let part = self.map.get(key);
        part.unwrap_or_else(|| panic!("no entry {key:?}"))
    }
}

impl<V, E> TrackedMap<V, E> {
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn get(&self, key: &str) -> Option<&V> {
        self.map.get(key).map(|part| &**part)
    }

    pub fn contains_key(&self, key: &str) -> bool {
        self.map.contains_key(key)
    }

    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.map.keys()
    }

    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.map.values().map(|part| &**part)
    }

    /// Every entry, in key order.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &V)> {
        self.map.iter().map(|(key, part)| (key, &**part))
    }

    pub(super) fn tracked_mut(&mut self, key: &str) -> Option<&mut Tracked<V, E>> {
        self.map.get_mut(key)
    }

    pub(super) fn values_mut(&mut self) -> impl Iterator<Item = &mut Tracked<V, E>> {
        self.map.values_mut()
    }

    /// Put `part` under `key`, with the undo it holds.
    pub(super) fn put(&mut self, key: String, part: Tracked<V, E>) {
        self.map.insert(key, part);
    }

    /// Take the entry under `key` off, with the undo it holds.
    pub(super) fn take(&mut self, key: &str) -> Option<Tracked<V, E>> {
        self.map.remove(key)
    }
}

/// One kind of component on every entity of one kind, such as every actor's meters: each
/// entity's own map, by its id. An entity with none has no entry, so a store reads the same
/// after an add is rolled back.
#[derive(Serialize)]
#[serde(transparent, bound(serialize = "V: Serialize"))]
pub struct Store<V, E> {
    on: BTreeMap<String, TrackedMap<V, E>>,
}

impl<V, E> Default for Store<V, E> {
    fn default() -> Self {
        Self {
            on: BTreeMap::new(),
        }
    }
}

/// A copy holds the values alone, at their frontier, as `Tracked` copies do.
impl<V: Clone, E> Clone for Store<V, E> {
    fn clone(&self) -> Self {
        Self {
            on: self.on.clone(),
        }
    }
}

impl<V: PartialEq, E> PartialEq for Store<V, E> {
    fn eq(&self, other: &Self) -> bool {
        self.on == other.on
    }
}

impl<V: Eq, E> Eq for Store<V, E> {}

impl<V: fmt::Debug, E> fmt::Debug for Store<V, E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.on.fmt(f)
    }
}

/// The save writes a store as each entity's id with its components by key:
/// `pc_rowan: {grit: {base: 1, offset: 0}}`. An entity listed with none has no entry.
impl<'de, V: Deserialize<'de>, E> Deserialize<'de> for Store<V, E> {
    fn deserialize<D: Deserializer<'de>>(from: D) -> Result<Self, D::Error> {
        let mut on = BTreeMap::<String, TrackedMap<V, E>>::deserialize(from)?;
        on.retain(|_, parts| !parts.is_empty());
        Ok(Store { on })
    }
}

impl<V, E> Store<V, E> {
    /// The components on entity `id`, by key, when it holds any.
    pub fn of(&self, id: &str) -> Option<&TrackedMap<V, E>> {
        self.on.get(id)
    }

    /// The component on entity `id` under `key`.
    pub fn get(&self, id: &str, key: &str) -> Option<&V> {
        self.on.get(id)?.get(key)
    }

    /// Every component, in entity and then key order.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &String, &V)> {
        let entities = self.on.iter();
        entities.flat_map(|(id, parts)| parts.iter().map(move |(key, part)| (id, key, part)))
    }

    pub(super) fn tracked_mut(&mut self, id: &str, key: &str) -> Option<&mut Tracked<V, E>> {
        self.on.get_mut(id)?.tracked_mut(key)
    }

    /// Put `part` on entity `id` under `key`, with the undo it holds.
    pub(super) fn put(&mut self, id: &str, key: String, part: Tracked<V, E>) {
        self.on.entry(id.to_string()).or_default().put(key, part);
    }

    /// Take the component on entity `id` under `key` off, with the undo it holds.
    pub(super) fn take(&mut self, id: &str, key: &str) -> Option<Tracked<V, E>> {
        let parts = self.on.get_mut(id)?;
        let part = parts.take(key);
        if parts.is_empty() {
            self.on.remove(id);
        }
        part
    }
}

/// One entity's components of one kind, to read: its own map, or none when it holds none.
pub struct Parts<'a, V, E>(pub(super) Option<&'a TrackedMap<V, E>>);

impl<'a, V, E> Parts<'a, V, E> {
    /// The component under `key`.
    pub fn get(&self, key: &str) -> Option<&'a V> {
        self.0?.get(key)
    }

    /// Every component, in key order.
    pub fn iter(&self) -> impl Iterator<Item = (&'a String, &'a V)> {
        self.0.into_iter().flat_map(TrackedMap::iter)
    }
}

/// One entity's components of one kind, to write: its entry in its kind's store, by its id,
/// or the world's own map.
pub(super) enum PartsMut<'a, V, E> {
    Stored(&'a mut Store<V, E>, &'a str),
    World(&'a mut TrackedMap<V, E>),
}

impl<'a, V, E> PartsMut<'a, V, E> {
    pub(super) fn tracked_mut(self, key: &str) -> Option<&'a mut Tracked<V, E>> {
        match self {
            PartsMut::Stored(store, id) => store.tracked_mut(id, key),
            PartsMut::World(map) => map.tracked_mut(key),
        }
    }

    pub(super) fn put(&mut self, key: String, part: Tracked<V, E>) {
        match self {
            PartsMut::Stored(store, id) => store.put(id, key, part),
            PartsMut::World(map) => map.put(key, part),
        }
    }

    pub(super) fn take(&mut self, key: &str) -> Option<Tracked<V, E>> {
        match self {
            PartsMut::Stored(store, id) => store.take(id, key),
            PartsMut::World(map) => map.take(key),
        }
    }
}

/// Marks with no value, such as flags, written as the list of their keys: `[calm, wary]`.
pub(crate) mod marks {
    use super::*;

    pub(crate) fn serialize<S: Serializer>(
        marks: &TrackedMap<(), ()>,
        out: S,
    ) -> Result<S::Ok, S::Error> {
        out.collect_seq(marks.keys())
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
        from: D,
    ) -> Result<TrackedMap<(), ()>, D::Error> {
        let mut marks = TrackedMap::default();
        for key in Vec::<String>::deserialize(from)? {
            marks.put(key, Tracked::new(()));
        }
        Ok(marks)
    }

    /// A store of marks, written as each entity's id with the list of its keys:
    /// `mira: [calm, wary]`.
    pub(crate) mod by_entity {
        use super::super::*;

        pub(crate) fn serialize<S: Serializer>(
            store: &Store<(), ()>,
            out: S,
        ) -> Result<S::Ok, S::Error> {
            let entities = store.on.iter();
            out.collect_map(entities.map(|(id, marks)| (id, marks.keys().collect::<Vec<_>>())))
        }

        pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
            from: D,
        ) -> Result<Store<(), ()>, D::Error> {
            let mut store = Store::default();
            for (id, keys) in BTreeMap::<String, Vec<String>>::deserialize(from)? {
                for key in keys {
                    store.put(&id, key, Tracked::new(()));
                }
            }
            Ok(store)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::item::Shift;

    #[test]
    fn a_rebase_keeps_the_value_and_only_the_newest_undo() {
        let mut oil: Tracked<u32, Shift> = Tracked::new(1);
        assert!(oil.apply(Shift(2)));
        assert!(oil.apply(Shift(3)));
        assert!(!oil.apply(Shift(0)), "no move");
        oil.rebase(1);
        assert_eq!(*oil, 6);
        oil.rollback(0);
        assert_eq!(*oil, 3, "the older edit can no longer be undone");
        oil.rebase(0);
        assert_eq!(oil.edits(), 0);
    }
}
