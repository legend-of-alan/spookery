//! A value that keeps the undo of each edit made to it since its frontier, so a round can roll
//! it back; the store that keeps one such value per holder and key; and the edits the save's
//! lists and maps share.

use std::collections::BTreeMap;
use std::fmt;
use std::ops::Deref;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::rules::effect::Holder;

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

/// One kind of component, such as every live meter: at most one on each holder under each key.
/// A component's identity is that holder and key, the same in the save as in a round. Reading
/// gives values; only `State`'s ledger moves one in a round.
pub struct Store<V, E> {
    on: BTreeMap<Holder, BTreeMap<String, Tracked<V, E>>>,
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

/// The save writes a store as each holder, as a rule's `who` names it, with its components by
/// key: `pc_rowan: {grit: {base: 1, offset: 0}}`.
impl<V: Serialize, E> Serialize for Store<V, E> {
    fn serialize<S: Serializer>(&self, out: S) -> Result<S::Ok, S::Error> {
        out.collect_map(self.on.iter().map(|(holder, parts)| {
            let values: BTreeMap<&String, &V> =
                parts.iter().map(|(key, part)| (key, &**part)).collect();
            (holder.to_string(), values)
        }))
    }
}

impl<'de, V: Deserialize<'de>, E> Deserialize<'de> for Store<V, E> {
    fn deserialize<D: Deserializer<'de>>(from: D) -> Result<Self, D::Error> {
        let mut store = Store::default();
        for (holder, parts) in BTreeMap::<String, BTreeMap<String, V>>::deserialize(from)? {
            let holder: Holder = holder.parse().map_err(D::Error::custom)?;
            for (key, value) in parts {
                store.put(holder.clone(), key, Tracked::new(value));
            }
        }
        Ok(store)
    }
}

/// A store of marks with no value, such as flags, written as each holder with the list of its
/// keys: `mira: [calm, wary]`.
pub(crate) mod marks {
    use super::*;

    pub(crate) fn serialize<S: Serializer>(
        store: &Store<(), ()>,
        out: S,
    ) -> Result<S::Ok, S::Error> {
        out.collect_map(
            store
                .on
                .iter()
                .map(|(holder, parts)| (holder.to_string(), parts.keys().collect::<Vec<_>>())),
        )
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
        from: D,
    ) -> Result<Store<(), ()>, D::Error> {
        let mut store = Store::default();
        for (holder, keys) in BTreeMap::<String, Vec<String>>::deserialize(from)? {
            let holder: Holder = holder.parse().map_err(D::Error::custom)?;
            for key in keys {
                store.put(holder.clone(), key, Tracked::new(()));
            }
        }
        Ok(store)
    }
}

impl<V, E> Store<V, E> {
    /// Whether the store holds no component, for `skip_serializing_if`.
    pub fn is_empty(&self) -> bool {
        self.on.is_empty()
    }

    /// The component on `holder` under `key`.
    pub fn get(&self, holder: &Holder, key: &str) -> Option<&V> {
        self.on.get(holder)?.get(key).map(|part| &**part)
    }

    /// The components on `holder`, in key order.
    pub fn on(&self, holder: &Holder) -> impl Iterator<Item = (&String, &V)> {
        let parts = self.on.get(holder).into_iter().flatten();
        parts.map(|(key, part)| (key, &**part))
    }

    /// Every component, in holder and then key order.
    pub fn iter(&self) -> impl Iterator<Item = (&Holder, &String, &V)> {
        let holders = self.on.iter();
        holders
            .flat_map(|(holder, parts)| parts.iter().map(move |(key, part)| (holder, key, &**part)))
    }

    pub(super) fn tracked_mut(&mut self, holder: &Holder, key: &str) -> Option<&mut Tracked<V, E>> {
        self.on.get_mut(holder)?.get_mut(key)
    }

    /// Put `part` on `holder` under `key`, with the undo it holds.
    pub(super) fn put(&mut self, holder: Holder, key: String, part: Tracked<V, E>) {
        self.on.entry(holder).or_default().insert(key, part);
    }

    /// Take the component on `holder` under `key` off, with the undo it holds. A holder left
    /// with none has no entry, so a store reads the same after an add is rolled back.
    pub(super) fn take(&mut self, holder: &Holder, key: &str) -> Option<Tracked<V, E>> {
        let parts = self.on.get_mut(holder)?;
        let part = parts.remove(key);
        if parts.is_empty() {
            self.on.remove(holder);
        }
        part
    }

    /// Take every component on `holder` off, with the undo each holds, in key order.
    pub(super) fn take_all(&mut self, holder: &Holder) -> BTreeMap<String, Tracked<V, E>> {
        self.on.remove(holder).unwrap_or_default()
    }
}

/// A collection round code reads but cannot change: only `State`'s writers put an entry in or
/// take one out, through the ledger. It reads as the collection it holds.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ReadOnly<C>(C);

impl<C> Deref for ReadOnly<C> {
    type Target = C;

    fn deref(&self) -> &C {
        &self.0
    }
}

impl<'a, C> IntoIterator for &'a ReadOnly<C>
where
    &'a C: IntoIterator,
{
    type Item = <&'a C as IntoIterator>::Item;
    type IntoIter = <&'a C as IntoIterator>::IntoIter;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<C> ReadOnly<C> {
    /// The collection, to change: for `State`'s writers alone.
    pub(super) fn get_mut(&mut self) -> &mut C {
        &mut self.0
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
