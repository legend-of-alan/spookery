//! The round a save is being written in: its limits, who is seated for it, and what its
//! writes owe the save: the keys counted toward the budget, each change for the next take, each
//! component they touched, for a rollback, and the memory lines they wrote.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::canon::Budget;
use crate::model::key::{Change, StateKey, StateValue};
use crate::model::modifier::ModifierLive;
use crate::model::tracked::{Edit, Tracked};
use crate::model::{ComponentId, MemoryLine, Presence, State};
use crate::rules::effect::Holder;
use crate::Reason;

/// The one ledger of a round, kept on its save. Out of a round it has no limits and nobody is
/// seated. A copy of the save holds no round, so a copy of a ledger is one at rest, and two
/// saves are equal whatever their ledgers hold.
pub(crate) struct Ledger {
    max_keys: usize,
    max_memory_writes: u64,
    /// Who is seated for the round. The host's seats own it; the save never holds it.
    presence: Presence,
    /// The keys counted toward the budget this round.
    keys: BTreeSet<StateKey>,
    /// The same keys in the order they were counted, so a savepoint can give them back.
    key_order: Vec<StateKey>,
    /// Each component a write changed, put on or took off, oldest first.
    touched: Vec<Touch>,
    /// Each key a write moved, with its value before and after that write, oldest first.
    changes: Vec<Change>,
    /// How many changes the last take had reported.
    taken: usize,
    memories: Vec<MemoryLine>,
    memory_writes: u32,
}

impl Default for Ledger {
    fn default() -> Self {
        Self {
            max_keys: usize::MAX,
            max_memory_writes: u64::MAX,
            presence: Presence::default(),
            keys: BTreeSet::new(),
            key_order: Vec::new(),
            touched: Vec::new(),
            changes: Vec::new(),
            taken: 0,
            memories: Vec::new(),
            memory_writes: 0,
        }
    }
}

impl Clone for Ledger {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl PartialEq for Ledger {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Eq for Ledger {}

impl fmt::Debug for Ledger {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Ledger")
    }
}

/// A component a write changed, with how many edits it held before; one a write put on; or
/// one it took off. A rollback undoes each touch since its savepoint, newest first.
enum Touch {
    Edited(ComponentId, usize),
    Added(ComponentId),
    Removed(Removed),
}

/// A component a write took off, whole, with the undo it held, to put back.
pub(crate) enum Removed {
    Modifier(Holder, String, Tracked<ModifierLive, ModifierLive>),
    /// A flag holds no value, so its holder and key are all there is.
    Flag(Holder, String),
}

/// How far the round had come: the length of each log a rollback cuts back.
#[derive(Clone, Copy)]
pub(crate) struct Savepoint {
    touched: usize,
    keys: usize,
    changes: usize,
    taken: usize,
    memories: usize,
    memory_writes: u32,
}

impl Ledger {
    /// A ledger for a round under `budget`, with `presence` seated.
    pub(crate) fn new(budget: &Budget, presence: &Presence) -> Self {
        Self {
            max_keys: budget.max_state_keys_mutated as usize,
            max_memory_writes: budget.max_memory_writes,
            presence: presence.clone(),
            ..Self::default()
        }
    }

    /// Make `edit` on `part`, which `at` names, and keep how many edits it held before, for a
    /// rollback. Whether it moved.
    pub(crate) fn track<T, E: Edit<T>>(
        &mut self,
        at: impl FnOnce() -> ComponentId,
        part: &mut Tracked<T, E>,
        edit: E,
    ) -> bool {
        let edits = part.edits();
        let moved = part.apply(edit);
        if moved {
            self.touched.push(Touch::Edited(at(), edits));
        }
        moved
    }

    /// Make `edit` on `part`, and when it moved, count `key` and keep its values before and
    /// after, as `read` reads them.
    pub(crate) fn land<T, E: Edit<T>>(
        &mut self,
        at: impl FnOnce() -> ComponentId,
        part: &mut Tracked<T, E>,
        edit: E,
        key: StateKey,
        read: impl Fn(&T, &StateKey) -> StateValue,
    ) -> Result<(), Reason> {
        let before = read(part, &key);
        if !self.track(at, part, edit) {
            return Ok(());
        }
        let after = read(part, &key);
        self.note(key, before, after)
    }

    /// Keep that a write put on the component `at` names, for a rollback to take off.
    pub(crate) fn added(&mut self, at: ComponentId) {
        self.touched.push(Touch::Added(at));
    }

    /// Keep a component a write took off, for a rollback to put back.
    pub(crate) fn removed(&mut self, removed: Removed) {
        self.touched.push(Touch::Removed(removed));
    }

    /// Count `key` and keep its change for the next take.
    pub(crate) fn note(
        &mut self,
        key: StateKey,
        before: StateValue,
        after: StateValue,
    ) -> Result<(), Reason> {
        self.count_key(&key)?;
        self.keep(Change { key, before, after });
        Ok(())
    }

    /// Keep a change whose key is already counted, for the next take.
    pub(crate) fn keep(&mut self, change: Change) {
        self.changes.push(change);
    }

    /// Count `key` toward the budget the first time this round, whether or not a write
    /// changed it: a cost of 0 still takes a key.
    pub(crate) fn count_key(&mut self, key: &StateKey) -> Result<(), Reason> {
        if self.keys.contains(key) {
            return Ok(());
        }
        if self.keys.len() >= self.max_keys {
            return Err(Reason::Budget);
        }
        self.keys.insert(key.clone());
        self.key_order.push(key.clone());
        Ok(())
    }

    /// Count one memory write, up to the round's limit.
    pub(crate) fn count_memory_write(&mut self) -> Result<(), Reason> {
        if u64::from(self.memory_writes) >= self.max_memory_writes {
            return Err(Reason::Budget);
        }
        self.memory_writes += 1;
        Ok(())
    }

    pub(crate) fn push_memory(&mut self, line: MemoryLine) {
        self.memories.push(line);
    }

    /// The memory lines the round wrote. Taking them undoes nothing.
    pub(crate) fn take_memories(&mut self) -> Vec<MemoryLine> {
        std::mem::take(&mut self.memories)
    }
}

impl State {
    /// Who is seated for the round this save is being written in.
    pub(crate) fn presence(&self) -> &Presence {
        &self.ledger.presence
    }

    /// Where the round has come to, to roll back to. It copies nothing.
    pub(crate) fn savepoint(&self) -> Savepoint {
        let ledger = &self.ledger;
        Savepoint {
            touched: ledger.touched.len(),
            keys: ledger.key_order.len(),
            changes: ledger.changes.len(),
            taken: ledger.taken,
            memories: ledger.memories.len(),
            memory_writes: ledger.memory_writes,
        }
    }

    /// Undo everything since `point`: undo each touch since, newest first, and give back the
    /// keys, changes and memories added since.
    pub(crate) fn rollback(&mut self, point: Savepoint) {
        let touched: Vec<Touch> = self.ledger.touched.drain(point.touched..).collect();
        for touch in touched.into_iter().rev() {
            match touch {
                Touch::Edited(at, edits) => {
                    if let Some(part) = self.component_mut(&at) {
                        part.rollback(edits);
                    }
                }
                Touch::Added(at) => self.take_off(&at),
                Touch::Removed(Removed::Modifier(holder, id, part)) => {
                    self.modifiers.put(holder, id, part);
                }
                Touch::Removed(Removed::Flag(holder, key)) => {
                    self.flags.put(holder, key, Tracked::default());
                }
            }
        }
        let ledger = &mut self.ledger;
        for key in ledger.key_order.drain(point.keys..) {
            ledger.keys.remove(&key);
        }
        ledger.changes.truncate(point.changes);
        ledger.taken = point.taken;
        ledger.memories.truncate(point.memories);
        ledger.memory_writes = point.memory_writes;
    }

    /// Accept the round: nothing it wrote can be rolled back now, so each component it touched
    /// keeps no undo, and the ledger starts over.
    pub(crate) fn rebase(&mut self) {
        let ledger = std::mem::take(&mut self.ledger);
        for touch in ledger.touched {
            if let Touch::Edited(at, _) | Touch::Added(at) = touch {
                if let Some(part) = self.component_mut(&at) {
                    part.rebase(0);
                }
            }
        }
    }

    /// Every key whose value differs from what it was at the last take, sorted: its value
    /// before its first change since then, and after its last.
    pub(crate) fn take_changes(&mut self) -> Vec<Change> {
        let ledger = &mut self.ledger;
        let since = std::mem::replace(&mut ledger.taken, ledger.changes.len());
        let mut net: BTreeMap<&StateKey, Change> = BTreeMap::new();
        for change in &ledger.changes[since..] {
            net.entry(&change.key)
                .and_modify(|first| first.after = change.after.clone())
                .or_insert_with(|| change.clone());
        }
        net.into_values()
            .filter(|change| change.before != change.after)
            .collect()
    }

    /// How many keys the round has counted so far.
    #[cfg(test)]
    pub(crate) fn keys_spent(&self) -> usize {
        self.ledger.key_order.len()
    }
}
