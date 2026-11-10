//! The round's transaction: the keys counted toward the budget, each change for the next take,
//! the memory lines, the change number each store's inverses are tagged with, and the savepoints
//! a refused action rolls back to.

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};

use super::change::{self, Change};
use super::{sort_unique, undo, MemoryLine, State};
use crate::canon::Budget;
use crate::Reason;

/// What the round's writes owe it: the keys counted toward the budget, in the order they were
/// counted, each change for the next take, the memory lines, and the number the next inverse is
/// tagged with. At rest it has no limits and no change made.
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
    memory_writes: u64,
    /// The number the next change's inverse is tagged with: how many changes the round made.
    change_no: u64,
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

    /// Count `change`'s key, then keep it for the next take. The write it reports has already
    /// landed, so a `BUDGET` here is for the caller's savepoint to undo. Only a store's write
    /// calls it, with the reports the writer handed the write.
    pub(super) fn report(&mut self, change: Change) -> Result<(), Reason> {
        self.count_key(&change.key)?;
        self.changes.push(change);
        Ok(())
    }

    /// The number to tag the next change's inverse with.
    pub(super) fn next_change_no(&mut self) -> u64 {
        self.change_no += 1;
        self.change_no - 1
    }
}

/// How far the round had come: the number the next change takes, and the length of each of the
/// transaction's logs.
#[derive(Clone, Copy)]
pub(crate) struct Savepoint {
    change_no: u64,
    keys: usize,
    changes: usize,
    taken: usize,
    memories: usize,
    memory_writes: u64,
}

impl State {
    /// Open a round on this save under `budget`'s limits. Opening it on writes not yet accepted
    /// would forget what the round counted, so that panics in every build.
    pub(crate) fn open_round(&mut self, budget: &Budget) {
        let transaction = &mut self.transaction;
        assert!(
            transaction.change_no == 0,
            "a round opened on unaccepted writes"
        );
        transaction.limits = Some(Limits {
            keys: usize::try_from(budget.max_state_keys_mutated).unwrap_or(usize::MAX),
            memory_writes: budget.max_memory_writes,
        });
    }

    pub(crate) fn savepoint(&self) -> Savepoint {
        let transaction = &self.transaction;
        Savepoint {
            change_no: transaction.change_no,
            keys: transaction.key_order.len(),
            changes: transaction.changes.len(),
            taken: transaction.taken,
            memories: transaction.memories.len(),
            memory_writes: transaction.memory_writes,
        }
    }

    /// Undo, in every store, each change made since `point`, and cut the transaction back to it.
    /// The round's limits stay.
    pub(crate) fn rollback(&mut self, point: Savepoint) {
        for log in self.logs_mut() {
            log.undo_to(point.change_no);
        }
        let transaction = &mut self.transaction;
        transaction.change_no = point.change_no;
        for key in transaction.key_order.drain(point.keys..) {
            transaction.keys.remove(&key);
        }
        transaction.changes.truncate(point.changes);
        transaction.taken = point.taken;
        transaction.memories.truncate(point.memories);
        transaction.memory_writes = point.memory_writes;
    }

    /// Every kind's stores and the notes, as the logs a rollback undoes and an accept clears.
    fn logs_mut(&mut self) -> impl Iterator<Item = &mut dyn undo::Log> {
        let notes: &mut dyn undo::Log = &mut self.notes;
        (self.actors.logs_mut().into_iter())
            .chain(self.places.logs_mut())
            .chain(self.stories.logs_mut())
            .chain(self.world.logs_mut())
            .chain([notes])
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
        if transaction.memory_writes >= limit {
            return Err(Reason::Budget);
        }
        transaction.memory_writes += 1;
        Ok(())
    }

    pub(crate) fn push_memory(&mut self, line: MemoryLine) {
        self.transaction.memories.push(line);
    }

    /// Close the round: every store forgets its inverses, the transaction goes back to rest, and
    /// the save into file order.
    pub fn accept(&mut self) {
        for log in self.logs_mut() {
            log.clear();
        }
        self.transaction = Transaction::default();
        let notes = self.notes.value_mut();
        notes.sort_by(|a, b| a.id.cmp(&b.id));
        for note in notes {
            sort_unique(&mut note.to);
            sort_unique(&mut note.fact_ids);
        }
        for known in self.actors.known_facts.value_mut().values_mut() {
            sort_unique(known);
        }
    }
}

#[cfg(test)]
impl State {
    /// How many keys the round has counted toward the budget.
    pub(crate) fn keys_counted(&self) -> usize {
        self.transaction.key_order.len()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::intent::Intent;
    use crate::rules::effect::{Effect, EffectKind};
    use crate::state::id::{ActorId, EntityId};
    use crate::state::meter::{Meter, MeterWrite};
    use crate::state::{ActorKind, Presence, State};
    use crate::{Reason, World};

    fn docks() -> World<()> {
        let pack =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packs/oneshot-docks");
        World::open(&pack).expect("open")
    }

    #[test]
    fn a_key_counts_toward_the_budget_only_once_a_write_changes_it() {
        let opened = docks();
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
        let world = World::from_parts(opened.root(), Arc::new(canon), state);
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

    #[test]
    fn a_rollback_undoes_the_writes_after_its_savepoint_and_keeps_those_before() {
        let world = docks();
        let (canon, mut state) = (world.canon(), world.begin_round());
        let mira = ActorId::from("mira");
        let rest = |state: &mut State, actor: &ActorId, delta: i64| {
            let shifted = state.actors_mut().shift_meter(canon, actor, "rest", delta);
            shifted.expect("rest moves");
        };
        // The write just before the savepoint stays; the one just after it goes.
        rest(&mut state, &mira, -1);
        let before = state.clone();
        let point = state.savepoint();
        rest(&mut state, &mira, -1);
        state.rollback(point);
        assert_eq!(
            state, before,
            "only the write after the savepoint is undone"
        );
        let changes = state.take_changes();
        let keys: Vec<_> = changes
            .iter()
            .map(|change| change.key.to_string())
            .collect();
        assert_eq!(
            keys,
            ["actor_meter mira rest"],
            "only the kept write is reported"
        );
    }

    #[test]
    fn a_rollback_takes_an_added_actor_off_every_store_it_was_put_on() {
        let world = docks();
        let mut state = world.begin_round();
        let before = state.clone();
        let point = state.savepoint();
        let nell = ActorId::from("nell");
        state
            .actors_mut()
            .add_actor(&nell, ActorKind::Npc, "docks_shop");
        assert!(state.actors().acts.contains_key(&nell), "Nell is added");
        state.rollback(point);
        assert_eq!(state, before, "her kind, place and action state all go");
    }
}
