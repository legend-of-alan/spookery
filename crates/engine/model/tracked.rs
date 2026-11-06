//! A value that keeps the undo of each edit made to it since its frontier, so a round can roll
//! it back; the store that keeps one such value per holder and key; and the edits the save's
//! lists and maps share.

use std::collections::BTreeMap;
use std::fmt;
use std::ops::Deref;

use crate::rules::effect::Holder;

/// A typed change to a `T`.
pub trait Edit<T>: Sized {
    /// Make this change to `value` and give back the edit that undoes it. `None` when `value`
    /// already was so, and then `value` is left as it was.
    fn apply(self, value: &mut T) -> Option<Self>;
}

/// A value, and the undo of each edit made to it since its frontier, oldest first. In a round
/// only an edit changes it. The save on disk holds the value alone.
pub struct Tracked<T, E> {
    value: T,
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
    pub(crate) fn edits(&self) -> usize {
        self.undo.len()
    }

    /// Keep the undo of only the newest `keep` edits. The value stays. `rebase(0)` makes the
    /// value the frontier.
    pub(crate) fn rebase(&mut self, keep: usize) {
        let older = self.undo.len().saturating_sub(keep);
        self.undo.drain(..older);
    }

    /// The value, to change in place at its frontier: outside a round, where nothing can be
    /// rolled back. A change here while an edit can still be undone would be lost to a
    /// rollback, so that panics.
    pub fn frontier_mut(&mut self) -> &mut T {
        assert!(
            self.undo.is_empty(),
            "a tracked value changed past its edits"
        );
        &mut self.value
    }
}

impl<T, E: Edit<T>> Tracked<T, E> {
    /// Make `edit` and keep its undo. Whether the value moved.
    pub(crate) fn apply(&mut self, edit: E) -> bool {
        let Some(undo) = edit.apply(&mut self.value) else {
            return false;
        };
        self.undo.push(undo);
        true
    }

    /// Undo the newest edits, newest first, until `edits` are left.
    pub(crate) fn rollback(&mut self, edits: usize) {
        for undo in self.undo.drain(edits..).rev() {
            // The edit it undoes was the newest one, so the value is as that edit left it.
            let redo = undo.apply(&mut self.value);
            debug_assert!(redo.is_some(), "an undo changed nothing");
        }
    }
}

/// A rollback or rebase of a tracked value, whatever it holds.
pub(crate) trait Rewind {
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

impl<V, E> Store<V, E> {
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

    /// Put `value` on `holder` under `key` at its frontier, outside a round, in place of any
    /// value there.
    pub fn insert(&mut self, holder: Holder, key: String, value: V) {
        self.put(holder, key, Tracked::new(value));
    }

    /// Take the component on `holder` under `key` off at its frontier, outside a round. Taking
    /// off one that can still be undone panics, as `Tracked::frontier_mut` does.
    pub fn remove(&mut self, holder: &Holder, key: &str) -> Option<V> {
        let mut part = self.take(holder, key)?;
        part.frontier_mut();
        Some(part.value)
    }

    /// The component on `holder` under `key`, to change at its frontier, as
    /// `Tracked::frontier_mut` says.
    pub fn frontier_mut(&mut self, holder: &Holder, key: &str) -> Option<&mut V> {
        Some(self.on.get_mut(holder)?.get_mut(key)?.frontier_mut())
    }

    /// Keep only the components `keep` accepts, outside a round.
    pub fn retain(&mut self, mut keep: impl FnMut(&Holder, &str, &V) -> bool) {
        for (holder, parts) in &mut self.on {
            parts.retain(|key, part| {
                part.frontier_mut();
                keep(holder, key, part)
            });
        }
        self.on.retain(|_, parts| !parts.is_empty());
    }

    pub(crate) fn tracked_mut(&mut self, holder: &Holder, key: &str) -> Option<&mut Tracked<V, E>> {
        self.on.get_mut(holder)?.get_mut(key)
    }

    /// Put `part` on `holder` under `key`, with the undo it holds.
    pub(crate) fn put(&mut self, holder: Holder, key: String, part: Tracked<V, E>) {
        self.on.entry(holder).or_default().insert(key, part);
    }

    /// Take the component on `holder` under `key` off, with the undo it holds. A holder left
    /// with none has no entry, so a store reads the same after an add is rolled back.
    pub(crate) fn take(&mut self, holder: &Holder, key: &str) -> Option<Tracked<V, E>> {
        let parts = self.on.get_mut(holder)?;
        let part = parts.remove(key);
        if parts.is_empty() {
            self.on.remove(holder);
        }
        part
    }
}

/// Map entry `id` takes `value`, `None` being no entry. Its undo holds the entry it replaced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry<V> {
    pub(crate) id: String,
    pub(crate) value: Option<V>,
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

/// A count in a map moves by `by`, a count of 0 being no entry. The writer has checked the
/// count it moves to. Its undo moves it back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Count {
    pub(crate) id: String,
    pub(crate) by: i64,
}

impl Edit<BTreeMap<String, u32>> for Count {
    fn apply(self, counts: &mut BTreeMap<String, u32>) -> Option<Self> {
        let held = counts.get(&self.id).copied().unwrap_or(0);
        let next = u32::try_from(i64::from(held).checked_add(self.by)?).ok()?;
        if next == held {
            return None;
        }
        if next == 0 {
            counts.remove(&self.id);
        } else {
            counts.insert(self.id.clone(), next);
        }
        Some(Count {
            id: self.id,
            by: -self.by,
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

    #[test]
    fn a_rebase_keeps_the_value_and_only_the_newest_undo() {
        let count = |id: &str, by| Count { id: id.into(), by };
        let mut items: Tracked<BTreeMap<String, u32>, Count> = Tracked::default();
        assert!(items.apply(count("oil", 2)));
        assert!(items.apply(count("rope", 1)));
        assert!(!items.apply(count("rope", 0)), "no move");
        items.rebase(1);
        assert_eq!(
            *items,
            BTreeMap::from([("oil".into(), 2), ("rope".into(), 1)])
        );
        items.rollback(0);
        let left = BTreeMap::from([("oil".into(), 2)]);
        assert_eq!(*items, left, "the older edit can no longer be undone");
        items.rebase(0);
        assert_eq!(items.edits(), 0);
    }
}
