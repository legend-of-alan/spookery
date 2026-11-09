//! The round's transaction: the keys counted toward the budget, each change for the next take,
//! the memory lines, and the savepoints a refused action rolls back to.

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};

use super::change::{self, Change};
use super::{
    sort_unique, ActorComponents, MemoryLine, Note, PlaceComponents, State, StoryComponents,
    WorldComponents,
};
use crate::canon::Budget;
use crate::Reason;

/// What the round's writes owe it: the keys counted toward the budget, in the order they were
/// counted, each change for the next take, and the memory lines. At rest it has no limits.
/// `Clone` gives one at rest and any two are equal: a copy of the save holds no round.
#[derive(Debug, Default)]
pub(crate) struct Transaction {
    /// The round's limits, or `None` at rest.
    limits: Option<Limits>,
    keys: BTreeSet<change::Key>,
    key_order: Vec<change::Key>,
    changes: Vec<Change>,
    taken: usize,
    memories: Vec<MemoryLine>,
    memory_writes: u32,
}

#[derive(Clone, Copy, Debug)]
struct Limits {
    keys: usize,
    memory_writes: u64,
}

impl Clone for Transaction {
    fn clone(&self) -> Self {
        Transaction::default()
    }
}

impl PartialEq for Transaction {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Eq for Transaction {}

impl Transaction {
    /// Count `key` toward the budget: `BUDGET` when it is new and the round's keys are spent.
    pub(super) fn count_key(&mut self, key: &change::Key) -> Result<(), Reason> {
        if self.keys.contains(key) {
            return Ok(());
        }
        if self
            .limits
            .is_some_and(|limits| self.keys.len() >= limits.keys)
        {
            return Err(Reason::Budget);
        }
        self.keys.insert(key.clone());
        self.key_order.push(key.clone());
        Ok(())
    }

    /// Count `key`, then keep its change for the next take. The write it reports has already
    /// landed, so a `BUDGET` here is for the caller's savepoint to undo.
    pub(super) fn report(
        &mut self,
        key: change::Key,
        before: change::Value,
        after: change::Value,
    ) -> Result<(), Reason> {
        self.count_key(&key)?;
        self.changes.push(Change { key, before, after });
        Ok(())
    }
}

/// The parts as they stood, and how far the round's transaction had come.
pub(crate) struct Savepoint {
    parts: (
        ActorComponents,
        PlaceComponents,
        StoryComponents,
        WorldComponents,
        Vec<Note>,
    ),
    keys: usize,
    changes: usize,
    taken: usize,
    memories: usize,
    memory_writes: u32,
}

impl State {
    /// Open a round on this save under `budget`'s limits. Opening it again would forget what the
    /// round counted, so that panics in every build.
    pub(crate) fn open_round(&mut self, budget: &Budget) {
        let transaction = &mut self.transaction;
        assert!(transaction.limits.is_none(), "a round opened twice");
        transaction.limits = Some(Limits {
            keys: usize::try_from(budget.max_state_keys_mutated).unwrap_or(usize::MAX),
            memory_writes: budget.max_memory_writes,
        });
    }

    pub(crate) fn savepoint(&self) -> Savepoint {
        let transaction = &self.transaction;
        Savepoint {
            parts: (
                self.actors.clone(),
                self.places.clone(),
                self.stories.clone(),
                self.world.clone(),
                self.notes.clone(),
            ),
            keys: transaction.key_order.len(),
            changes: transaction.changes.len(),
            taken: transaction.taken,
            memories: transaction.memories.len(),
            memory_writes: transaction.memory_writes,
        }
    }

    /// Put the parts back as `point` found them, and cut the transaction back to it. The round's
    /// limits stay.
    pub(crate) fn rollback(&mut self, point: Savepoint) {
        (
            self.actors,
            self.places,
            self.stories,
            self.world,
            self.notes,
        ) = point.parts;
        let transaction = &mut self.transaction;
        for key in transaction.key_order.drain(point.keys..) {
            transaction.keys.remove(&key);
        }
        transaction.changes.truncate(point.changes);
        transaction.taken = point.taken;
        transaction.memories.truncate(point.memories);
        transaction.memory_writes = point.memory_writes;
    }

    /// The changes since the last take, one per key from its first value to its last, less the
    /// keys that came back to where they were, in key order. The counted keys stay.
    pub(crate) fn take_changes(&mut self) -> Vec<Change> {
        let transaction = &mut self.transaction;
        let mut net = BTreeMap::new();
        for change in transaction.changes[transaction.taken..].iter().cloned() {
            match net.entry(change.key.clone()) {
                Entry::Vacant(entry) => {
                    entry.insert(change);
                }
                Entry::Occupied(mut entry) => entry.get_mut().after = change.after,
            }
        }
        transaction.taken = transaction.changes.len();
        let moved = net
            .into_values()
            .filter(|change| change.before != change.after);
        moved.collect()
    }

    pub(crate) fn take_memories(&mut self) -> Vec<MemoryLine> {
        std::mem::take(&mut self.transaction.memories)
    }

    /// Count `key` with no write of its own: a cost of 0 still takes a key of the budget.
    pub(crate) fn count_key(&mut self, key: &change::Key) -> Result<(), Reason> {
        self.transaction.count_key(key)
    }

    /// Count one memory write: `BUDGET` past the round's limit.
    pub(crate) fn count_memory_write(&mut self) -> Result<(), Reason> {
        let transaction = &mut self.transaction;
        let limit = transaction
            .limits
            .map_or(u64::MAX, |limits| limits.memory_writes);
        if u64::from(transaction.memory_writes) >= limit {
            return Err(Reason::Budget);
        }
        transaction.memory_writes += 1;
        Ok(())
    }

    pub(crate) fn push_memory(&mut self, line: MemoryLine) {
        self.transaction.memories.push(line);
    }

    /// Close the round: the transaction goes back to rest, and the save into file order.
    pub fn accept(&mut self) {
        self.transaction = Transaction::default();
        self.notes.sort_by(|a, b| a.id.cmp(&b.id));
        for note in &mut self.notes {
            sort_unique(&mut note.to);
            sort_unique(&mut note.fact_ids);
        }
        for known in self.actors.known_facts.values_mut() {
            sort_unique(known);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::intent::Intent;
    use crate::rules::effect::{Effect, EffectKind};
    use crate::state::id::{ActorId, EntityId};
    use crate::state::meter::{Meter, MeterWrite};
    use crate::state::Presence;
    use crate::{Reason, World};

    #[test]
    fn a_key_counts_toward_the_budget_only_once_a_write_changes_it() {
        let pack =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packs/oneshot-docks");
        let opened = World::open(&pack).expect("open");
        let mut canon = opened.canon().clone();
        canon.budget.max_state_keys_mutated = 2;
        let mut state = opened.state().clone();
        // Bram's grit sits clamped at its minimum: taking more moves the offset, not the value.
        let bram = ActorId::from("bram");
        let grit = Meter {
            base: 1,
            offset: -5,
        };
        state
            .actors_mut()
            .set_meter(&canon, &bram, "grit", Some(grit))
            .unwrap();
        state.accept();
        let world = World::from_parts(&pack, Arc::new(canon), state);
        let (canon, mut state, nobody) = (world.canon(), world.begin_round(), Presence::default());
        let intent = Intent::habit(world.state(), &"mira".into(), "leave_door_note");
        let mut write = |actor: &str, key: &str, delta: i64| {
            let kind = EffectKind::Meter {
                entity: EntityId::Actor(actor.into()),
                key: key.to_string(),
                write: MeterWrite::Offset(delta),
            };
            let effect = Effect::new(intent.actor.as_ref(), &intent.id, kind);
            effect.apply(canon, &mut state, &nobody, &intent)
        };
        assert_eq!(write("mira", "rest", 0), Ok(()), "unchanged: no key");
        assert_eq!(write("mira", "rest", 1), Ok(()), "the first key");
        assert_eq!(write("bram", "grit", -1), Ok(()), "offset moved: key two");
        assert_eq!(write("mira", "rest", 1), Ok(()), "a counted key is free");
        assert_eq!(write("aldric", "rest", 0), Ok(()), "unchanged: no key");
        assert_eq!(write("aldric", "rest", 1), Err(Reason::Budget));
    }
}
