//! A value that keeps the undo of each edit made to it since its frontier, so a round can roll
//! it back, and the edits the save's lists and maps share.

use std::collections::BTreeMap;
use std::fmt;
use std::ops::Deref;

use serde::{Deserialize, Serialize};

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

/// Whether a tracked list is empty, for `skip_serializing_if`.
pub(crate) fn is_empty<V, E>(list: &Tracked<Vec<V>, E>) -> bool {
    list.is_empty()
}

/// Whether a tracked map is empty, for `skip_serializing_if`.
pub(crate) fn is_empty_map<V, E>(map: &Tracked<BTreeMap<String, V>, E>) -> bool {
    map.is_empty()
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

/// A name joins a list of names, such as a holder's flags, at the end; leaves it; or comes
/// back where it was, which is how a leave is undone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Member {
    Join(String),
    Leave(String),
    Back(usize, String),
}

impl Member {
    /// The edit that sets `id` in or out of the list.
    pub(crate) fn set(id: &str, on: bool) -> Member {
        let id = id.to_string();
        if on {
            Member::Join(id)
        } else {
            Member::Leave(id)
        }
    }

    /// The name this edit moves.
    pub(crate) fn id(&self) -> &str {
        match self {
            Member::Join(id) | Member::Leave(id) | Member::Back(_, id) => id,
        }
    }
}

impl Edit<Vec<String>> for Member {
    fn apply(self, list: &mut Vec<String>) -> Option<Self> {
        let at = list.iter().position(|name| name == self.id());
        match (self, at) {
            (Member::Join(id), None) => {
                list.push(id.clone());
                Some(Member::Leave(id))
            }
            (Member::Back(at, id), None) => {
                list.insert(at, id.clone());
                Some(Member::Leave(id))
            }
            (Member::Leave(id), Some(at)) => {
                list.remove(at);
                Some(Member::Back(at, id))
            }
            _ => None,
        }
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
        let mut flags: Tracked<Vec<String>, Member> = Tracked::new(vec!["a".into(), "b".into()]);
        assert!(flags.apply(Member::set("a", false)));
        assert!(flags.apply(Member::set("c", true)));
        assert!(!flags.apply(Member::set("c", true)), "already in");
        flags.rebase(1);
        assert_eq!(*flags, ["b", "c"]);
        flags.rollback(0);
        assert_eq!(*flags, ["b"], "the older edit can no longer be undone");
        flags.rebase(0);
        assert_eq!(flags.edits(), 0);
    }
}
