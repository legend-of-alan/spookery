//! The round's transaction: the keys counted toward the budget, each change for the next take,
//! the memory lines, the change number each store's inverses are tagged with, and the savepoints
//! a refused action rolls back to.

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};

use super::change::{self, Change, ChangeValue};
use super::component::{meter, Meter, Modifier};
use super::{entity, undo, MemoryLine, Reader};
use crate::canon::Budget;
use crate::ecs;
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
    changes: Vec<change::Report>,
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

    /// Count `report`'s key, then keep it for the next take. The write it reports has already
    /// landed, so a `BUDGET` here is for the caller's savepoint to undo. Only a store's write
    /// calls it, with the reports the writer handed the write.
    pub(super) fn report(&mut self, report: change::Report) -> Result<(), Reason> {
        self.count_key(report.key())?;
        self.changes.push(report);
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

impl ecs::World {
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
        for history in self.history_mut() {
            history.undo_to(point.change_no);
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

    /// Every collection's stores, as the histories a rollback undoes and an accept clears.
    fn history_mut(&mut self) -> impl Iterator<Item = &mut dyn undo::History> {
        let save = &mut self.save;
        (save.actors.history_mut().into_iter())
            .chain(save.places.history_mut())
            .chain(save.stories.history_mut())
            .chain(save.globals.history_mut())
    }

    /// The changes since the last take, one per key from its first value to its last, less the
    /// keys that came back to where they were, in key order. A meter reads as rules read it,
    /// before its first write since the last take and now. The counted keys stay.
    pub(crate) fn take_changes(&mut self) -> Vec<Change> {
        let transaction = &self.transaction;
        let mut net = BTreeMap::new();
        // Each meter's stored value and modifier sum before the first write to each since the
        // last take.
        let mut meters: BTreeMap<_, (Option<Option<&Meter>>, Option<_>)> = BTreeMap::new();
        for report in &transaction.changes[transaction.taken..] {
            match report {
                change::Report::Change(change) => match net.entry(change.key.clone()) {
                    Entry::Vacant(entry) => {
                        entry.insert(change.clone());
                    }
                    Entry::Occupied(mut entry) => entry.get_mut().after = change.after.clone(),
                },
                change::Report::Meter(key, held) => {
                    (meters.entry(key).or_default().0).get_or_insert(held.as_ref());
                }
                change::Report::Modifiers(key, sum) => {
                    (meters.entry(key).or_default().1).get_or_insert(*sum);
                }
            }
        }
        for (key, (held, sum)) in meters {
            net.insert(key.clone(), self.read_meter_change(key, held, sum));
        }
        self.transaction.taken = self.transaction.changes.len();
        let moved = net
            .into_values()
            .filter(|change| change.before != change.after);
        moved.collect()
    }

    /// Meter `key`'s change as rules read it: before, from `held` and `sum` where a write handed
    /// them over and from the meter as it is where none did; and as it is now.
    fn read_meter_change(
        &self,
        key: &change::Key,
        held: Option<Option<&Meter>>,
        sum: Option<Result<i64, Reason>>,
    ) -> Change {
        let (canon, save) = (&*self.canon, &self.save);
        let (id, entry_id) = (&key.entity_id, key.entry_id.as_deref().unwrap_or_default());
        let desc = canon.meter_desc(id.as_ref(), entry_id);
        let now = entity::in_collection!(id, [&save], |entities, id| {
            Reader::new(entities, canon).get::<Meter>(id).get(entry_id)
        });
        let sum_now = match id {
            entity::Id::Actor(actor) => self.actors().get::<Modifier>(actor).sum(entry_id),
            _ => Ok(0),
        };
        let reading = |held: Option<&Meter>, sum: Result<i64, Reason>| {
            let value = (desc.zip(held).ok_or(Reason::UnknownId))
                .and_then(|(desc, held)| meter::value(desc, held, sum?));
            value.ok().map(ChangeValue::Number)
        };
        let before = reading(held.unwrap_or(now), sum.unwrap_or(sum_now));
        Change::new(key.clone(), before, reading(now, sum_now))
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

    /// Close the round: every store forgets its inverses, and the transaction goes back to rest.
    pub fn accept(&mut self) {
        for history in self.history_mut() {
            history.clear();
        }
        self.transaction = Transaction::default();
    }
}

#[cfg(test)]
impl ecs::World {
    /// How many keys the round has counted toward the budget.
    pub(crate) fn keys_counted(&self) -> usize {
        self.transaction.key_order.len()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::ecs::component::meter::{self, Meter};
    use crate::ecs::id::{ActorId, PlaceId, StoryId};
    use crate::ecs::{self, Presence, Save};
    use crate::ecs::{component, entity};
    use crate::intent::Intent;
    use crate::rules::effect::{Effect, EffectKind};
    use crate::{session, Reason};

    fn docks() -> session::Session<()> {
        let pack = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/packs/docks");
        session::Session::open(&pack).expect("open")
    }

    #[test]
    fn a_key_counts_toward_the_budget_only_once_a_write_changes_it() {
        let opened = docks();
        let mut canon = opened.canon().clone();
        canon.budget.max_state_keys_mutated = 2;
        let mut world = ecs::World::new(Save::clone(opened.world()), Arc::new(canon));
        // Bram's vigor sits clamped at its minimum: taking more moves the offset, not the value.
        let bram = ActorId::from("bram");
        let vigor = Meter {
            base: 1,
            offset: -5,
        };
        let mut actors = world.actors_mut();
        actors
            .get_mut::<Meter>(&bram)
            .set("vigor", Some(vigor))
            .unwrap();
        world.accept();
        let session = session::Session::from_parts(opened.root(), world);
        let (canon, mut world, nobody) =
            (session.canon(), session.begin_round(), Presence::default());
        let intent = Intent::habit(session.world(), &"mira".into(), "leave_door_note");
        let mut write = |actor: &str, key: &str, delta: i64| {
            let kind = EffectKind::Meter {
                entity: entity::Id::Actor(actor.into()),
                key: key.to_string(),
                write: meter::Write::Offset(delta),
            };
            let effect = Effect::new(intent.actor.as_ref(), &intent.id, kind);
            effect.apply(canon, &mut world, &nobody, &intent)
        };
        assert_eq!(write("mira", "rest", 0), Ok(()), "unchanged: no key");
        assert_eq!(write("mira", "rest", 1), Ok(()), "the first key");
        assert_eq!(write("bram", "vigor", -1), Ok(()), "offset moved: key two");
        assert_eq!(write("mira", "rest", 1), Ok(()), "a counted key is free");
        assert_eq!(write("aldric", "rest", 0), Ok(()), "unchanged: no key");
        assert_eq!(write("aldric", "rest", 1), Err(Reason::Budget));
    }

    #[test]
    fn a_rollback_undoes_the_writes_after_its_savepoint_and_keeps_those_before() {
        let session = docks();
        let mut world = session.begin_round();
        let mira = ActorId::from("mira");
        let rest = |world: &mut ecs::World, actor: &ActorId, delta: i64| {
            let shifted = world
                .actors_mut()
                .get_mut::<Meter>(actor)
                .shift("rest", delta);
            shifted.expect("rest moves");
        };
        // The write just before the savepoint stays; the one just after it goes.
        rest(&mut world, &mira, -1);
        let before = world.clone();
        let point = world.savepoint();
        rest(&mut world, &mira, -1);
        world.rollback(point);
        assert_eq!(
            *world, *before,
            "only the write after the savepoint is undone"
        );
        let changes = world.take_changes();
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
    fn a_meter_that_comes_back_inside_one_take_is_not_taken_and_stays_counted() {
        use crate::ecs::component::modifier::{self, Modifier};
        let opened = docks();
        let mut canon = opened.canon().clone();
        let fatigue = modifier::Descriptor {
            meter: "rest".to_string(),
            max_stacks: 1,
            hidden: false,
        };
        canon.modifiers.insert("fatigue".to_string(), fatigue);
        let save = Save::clone(opened.world());
        let session =
            session::Session::from_parts(opened.root(), ecs::World::new(save, Arc::new(canon)));
        let mut world = session.begin_round();
        let mira = ActorId::from("mira");
        // A: down one, B: back up one, in one take window.
        for delta in [-1, 1] {
            let shifted = world
                .actors_mut()
                .get_mut::<Meter>(&mira)
                .shift("rest", delta);
            shifted.expect("rest moves");
        }
        assert_eq!(modifier::value(world.actors(), &mira, "rest"), Ok(3));
        assert_eq!(
            world.take_changes(),
            [],
            "the key that came back is dropped"
        );
        assert_eq!(world.keys_counted(), 1, "and still counted");
        // The same with a modifier that lowers rest, put on and taken off.
        let lowers = Modifier {
            meter: "rest".to_string(),
            until: 99,
            max_ticks: 99,
            stacks: vec![-2],
        };
        let put = (world.actors_mut().get_mut::<Modifier>(&mira)).set("fatigue", Some(lowers));
        put.unwrap();
        assert_eq!(modifier::value(world.actors(), &mira, "rest"), Ok(1));
        let taken = (world.actors_mut().get_mut::<Modifier>(&mira)).set("fatigue", None);
        taken.unwrap();
        let changes = world.take_changes();
        assert_eq!(changes, [], "the meter and the modifier are dropped");
        assert_eq!(
            world.keys_counted(),
            2,
            "the meter's key was counted already; the modifier's key is new"
        );
    }

    #[test]
    fn a_rollback_takes_an_added_actor_off_every_store_it_was_put_on() {
        let session = docks();
        let mut world = session.begin_round();
        let before = world.clone();
        let point = world.savepoint();
        let nell = ActorId::from("nell");
        world
            .actors_mut()
            .add_actor(&nell, component::ControlType::Npc, "docks_shop");
        let added = world.actors().get::<component::Act>(&nell).get().is_some();
        assert!(added, "Nell is added");
        world.rollback(point);
        assert_eq!(
            *world, *before,
            "her control type, place and action state all go"
        );
    }

    #[test]
    fn a_flag_goes_only_on_a_place_or_story_the_pack_defines() {
        let session = docks();
        let mut world = session.begin_round();
        let before = world.clone();
        let place = world
            .places_mut()
            .get_mut::<component::Flag>(&PlaceId::from("nowhere"))
            .set("lit", true);
        assert_eq!(
            place,
            Err(Reason::UnknownId),
            "the pack has no place nowhere"
        );
        let story = world
            .stories_mut()
            .get_mut::<component::Flag>(&StoryId::from("nowhere"))
            .set("lit", true);
        assert_eq!(
            story,
            Err(Reason::UnknownId),
            "the pack has no story nowhere"
        );
        assert_eq!(*world, *before, "the save is unchanged");
    }
}
