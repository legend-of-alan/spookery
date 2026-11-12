//! A store: one component's value, and the inverse of each change made to it since the last
//! accept, so a rollback puts the value back from the store alone.

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::ops::Deref;

use serde::{Deserialize, Serialize};

use super::change;
use super::id::ActorId;
use super::transaction::Transaction;
use crate::Reason;

/// A value a store holds, and how one change to it is undone.
pub trait Undo {
    /// What one change took away: enough to put the value back, and nothing else.
    type Inverse: Debug;
    fn undo(&mut self, inverse: Self::Inverse);
}

/// A map, written one entry at a time.
pub trait Entries {
    type Key: Ord + Clone + Debug;
    type Value: PartialEq + Debug;
    fn get(&self, key: &Self::Key) -> Option<&Self::Value>;
    /// Put `value` under `key`, `None` taking the entry off. What the entry held before. The key
    /// is cloned only for an entry that is new.
    fn put(&mut self, key: &Self::Key, value: Option<Self::Value>) -> Option<Self::Value>;
}

/// What one entry held before a write: `None` when it was not there.
#[derive(Debug)]
pub struct EntryInverse<K, V> {
    key: K,
    previous: Option<V>,
}

impl<T: Entries> Undo for T {
    type Inverse = EntryInverse<T::Key, T::Value>;
    fn undo(&mut self, inverse: Self::Inverse) {
        self.put(&inverse.key, inverse.previous);
    }
}

impl<K: Ord + Clone + Debug, V: PartialEq + Debug> Entries for BTreeMap<K, V> {
    type Key = K;
    type Value = V;
    fn get(&self, key: &K) -> Option<&V> {
        BTreeMap::get(self, key)
    }
    fn put(&mut self, key: &K, value: Option<V>) -> Option<V> {
        match (value, self.get_mut(key)) {
            (Some(value), Some(held)) => Some(std::mem::replace(held, value)),
            (Some(value), None) => self.insert(key.clone(), value),
            (None, _) => self.remove(key),
        }
    }
}

/// A store's log of inverses, as a rollback and an accept work through it.
pub trait Log {
    /// A rollback: undo every change numbered `change_no` or later, newest first.
    fn undo_to(&mut self, change_no: u64);
    /// An accept: drop every inverse, so nothing rolls the value back past here.
    fn clear(&mut self);
}

/// One component's value, and beside it the inverse of each change made since the last accept,
/// oldest first, each tagged with the change number the transaction gave it. The file holds the
/// value alone. Clones and comparisons see the value alone too: a copy holds no round.
#[derive(Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Store<T: Undo> {
    value: T,
    #[serde(skip)]
    log: Vec<(u64, T::Inverse)>,
}

impl<T: Undo + Clone> Clone for Store<T> {
    fn clone(&self) -> Self {
        Store::from(self.value.clone())
    }
}

impl<T: Undo + Default> Default for Store<T> {
    fn default() -> Self {
        Store::from(T::default())
    }
}

impl<T: Undo + PartialEq> PartialEq for Store<T> {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl<T: Undo + Eq> Eq for Store<T> {}

impl<T: Undo> From<T> for Store<T> {
    fn from(value: T) -> Self {
        Store {
            value,
            log: Vec::new(),
        }
    }
}

/// Reads; there is no `DerefMut`.
impl<T: Undo> Deref for Store<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.value
    }
}

impl<T: Undo> Store<T> {
    /// Make one change with `change`, which hands back its inverse; log the inverse under the
    /// transaction's next change number, then count and keep each of `reports` in order. A
    /// `BUDGET` from a report leaves the change made, for the caller's savepoint to undo.
    pub(super) fn record(
        &mut self,
        transaction: &mut Transaction,
        change: impl FnOnce(&mut T) -> T::Inverse,
        reports: impl IntoIterator<Item = change::Report>,
    ) -> Result<(), Reason> {
        let inverse = change(&mut self.value);
        self.log.push((transaction.next_change_no(), inverse));
        (reports.into_iter()).try_for_each(|report| transaction.report(report))
    }

    /// The value, to change past the log: only out of a round, so it panics while an inverse is
    /// held.
    pub(super) fn value_mut(&mut self) -> &mut T {
        assert!(self.log.is_empty(), "a store changed past its log");
        &mut self.value
    }
}

impl<T: Undo> Log for Store<T> {
    fn undo_to(&mut self, change_no: u64) {
        while let Some((_, inverse)) = self.log.pop_if(|(at, _)| *at >= change_no) {
            self.value.undo(inverse);
        }
    }

    fn clear(&mut self) {
        self.log.clear();
    }
}

impl<T: Entries> Store<T> {
    /// Set `value` under `key`, `None` taking the entry off. If the entry moves, its inverse is
    /// logged and `reports` are counted and kept, as `record` says; if it does not, nothing is,
    /// and `reports` are dropped unread. The one way an entry changes, and the one place its
    /// write reports.
    pub(super) fn set(
        &mut self,
        transaction: &mut Transaction,
        key: T::Key,
        value: Option<T::Value>,
        reports: impl IntoIterator<Item = change::Report>,
    ) -> Result<(), Reason> {
        if self.value.get(&key) == value.as_ref() {
            return Ok(());
        }
        let undo = |held: &mut T| EntryInverse {
            previous: held.put(&key, value),
            key,
        };
        self.record(transaction, undo, reports)
    }
}

/// Holds a value for every actor in `kind`. Production code can only set a value for an actor
/// already present; adding and taking one off are for the actor lifecycle only fixtures run.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DenseStore<V: PartialEq + Debug>(Store<BTreeMap<ActorId, V>>);

/// Reads; there is no `DerefMut`.
impl<V: PartialEq + Debug> Deref for DenseStore<V> {
    type Target = BTreeMap<ActorId, V>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<V: PartialEq + Debug> DenseStore<V> {
    /// Set `actor`'s value as `Store::set` does: `UNKNOWN_ID` for an actor with none.
    pub(super) fn set(
        &mut self,
        transaction: &mut Transaction,
        actor: &ActorId,
        value: V,
        reports: impl IntoIterator<Item = change::Report>,
    ) -> Result<(), Reason> {
        self.0.get(actor).ok_or(Reason::UnknownId)?;
        self.0.set(transaction, actor.clone(), Some(value), reports)
    }

    /// Give `actor` a value, or with `None` take theirs off: the actor lifecycle.
    #[cfg(any(test, feature = "testing"))]
    pub(super) fn put(&mut self, transaction: &mut Transaction, actor: &ActorId, value: Option<V>) {
        let put = self.0.set(transaction, actor.clone(), value, []);
        put.expect("a write that reports nothing is never refused");
    }
}

impl<V: PartialEq + Debug> Log for DenseStore<V> {
    fn undo_to(&mut self, change_no: u64) {
        self.0.undo_to(change_no);
    }

    fn clear(&mut self) {
        self.0.clear();
    }
}
