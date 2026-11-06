//! The staged save a round writes before commit.

use std::collections::BTreeSet;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::Arc;

use crate::canon::Budget;
use crate::canon::Canon;
use crate::model::key::{Change, StateKey};
use crate::model::record::{ActorId, Diff, PreImage, RecordId};
use crate::model::{ActCounter, MemoryLine, PendingAction, Presence, State};
use crate::world::World;
use crate::Reason;

pub(crate) struct Draft<'p> {
    canon: Arc<Canon>,
    pub(crate) state: State,
    /// Who is seated for this round. The round reads it and never writes it.
    pub(crate) presence: &'p Presence,
    /// The keys counted toward the budget this round.
    keys: BTreeSet<StateKey>,
    /// The same keys in the order they were counted, so a savepoint can give them back.
    key_order: Vec<StateKey>,
    /// Each record as it was just before a write to it, oldest first. A rollback puts back
    /// every image kept since its savepoint, newest first, and a take compares the first image
    /// of each record since the last take with the record now.
    pre_images: Vec<PreImage>,
    /// How many pre-images the last take had already seen.
    frontier: usize,
    pub(crate) memories: Vec<MemoryLine>,
    memory_writes: u32,
    _nosend: PhantomData<Rc<()>>,
}

impl<'p> Draft<'p> {
    /// Stage a round on a copy of the world's save, with `presence` seated.
    pub(crate) fn begin<L>(world: &World<L>, presence: &'p Presence) -> Self {
        Self {
            canon: Arc::clone(&world.canon),
            state: world.state.clone(),
            presence,
            keys: BTreeSet::new(),
            key_order: Vec::new(),
            pre_images: Vec::new(),
            frontier: 0,
            memories: Vec::new(),
            memory_writes: 0,
            _nosend: PhantomData,
        }
    }

    pub(crate) fn canon(&self) -> &Canon {
        &self.canon
    }

    pub(crate) fn shared_canon(&self) -> Arc<Canon> {
        Arc::clone(&self.canon)
    }

    pub(crate) fn state(&self) -> &State {
        &self.state
    }

    pub(crate) fn budget(&self) -> &Budget {
        &self.canon.budget
    }

    pub(crate) fn stamp_round(&mut self, round_index: u64, world_time: u64, last_round_at: &str) {
        self.state.round_index = round_index;
        self.state.world_time = world_time;
        self.state.last_round_at = last_round_at.to_string();
    }

    /// Set `counter` back to 0 on every actor.
    pub(crate) fn reset_acts(&mut self, counter: ActCounter) -> Result<(), Reason> {
        let ids: Vec<String> = self.state.actors.keys().cloned().collect();
        for id in ids {
            self.write(ActorId(&id), |actor| {
                *counter.count_mut(&mut actor.act) = 0;
                Ok(())
            })?;
        }
        Ok(())
    }

    /// Expire every modifier whose tick has come by `world_time`, all its stacks at once.
    pub(crate) fn expire_modifiers(&mut self, world_time: u64) -> Result<(), Reason> {
        let due: Vec<String> = self
            .state
            .actors
            .iter()
            .filter(|(_, actor)| {
                actor
                    .modifiers
                    .values()
                    .any(|held| held.until <= world_time)
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in due {
            self.write(ActorId(&id), |actor| {
                actor.modifiers.retain(|_, held| held.until > world_time);
                Ok(())
            })?;
        }
        Ok(())
    }

    /// Count one act for `actor_id` this round, in the act window the table is in.
    pub(crate) fn count_act(&mut self, actor_id: &str) -> Result<(), Reason> {
        let (round_index, players) = (self.state.round_index, self.presence.players);
        self.write(ActorId(actor_id), |actor| {
            let counters = &mut actor.act;
            counters.last_act_round = round_index;
            if players == 0 {
                counters.idle_acts_this_window = counters.idle_acts_this_window.saturating_add(1);
            } else {
                counters.live_acts_this_window = counters.live_acts_this_window.saturating_add(1);
            }
            Ok(())
        })
    }

    /// Keep actor `id` busy until `until`, on `pending` when it is an action under way and
    /// `None` when it is a cooldown.
    pub(crate) fn set_busy(
        &mut self,
        id: &str,
        until: u64,
        pending: Option<PendingAction>,
    ) -> Result<(), Reason> {
        self.write(ActorId(id), |actor| {
            actor.act.busy_until = Some(until);
            actor.act.pending = pending;
            Ok(())
        })
    }

    /// End actor `id`'s action under way, if any, and free them: it never finishes here.
    pub(crate) fn take_pending(&mut self, id: &str) -> Result<Option<PendingAction>, Reason> {
        let mut taken = None;
        self.write(ActorId(id), |actor| {
            taken = actor.act.pending.take();
            actor.act.busy_until = None;
            Ok(())
        })?;
        Ok(taken)
    }

    /// Write one record of the save. Its pre-image goes on the undo log, `write` runs on the
    /// record alone, and when `write` returns the record is compared with that image: every
    /// key whose stored fields changed is counted toward the key budget, in order, each checked
    /// before it is inserted, so a key that does not change is never counted. An `Err` from the
    /// budget leaves the write in place; the caller's savepoint undoes it. A meter's stored
    /// fields are its base and offset, which can move while the value the key reads holds, and
    /// hold while a modifier moves the value: only a take reports values.
    pub(crate) fn write<I: RecordId>(
        &mut self,
        id: I,
        write: impl FnOnce(&mut I::Record) -> Result<(), Reason>,
    ) -> Result<(), Reason> {
        let before = id.get(&self.state).cloned();
        let written = id.get_mut(&mut self.state).and_then(write);
        let mut diff = Diff::default();
        if written.is_ok() {
            if let Some(after) = id.get(&self.state) {
                id.diff(&self.canon, before.as_ref(), after, &mut diff);
            }
        }
        self.pre_images.push(id.pre_image(before));
        written?;
        diff.spent
            .into_iter()
            .try_for_each(|key| self.count_key(key))
    }

    /// Count `key` toward the budget the first time this round, whether or not a write changed
    /// it: a cost of 0 still takes a key.
    pub(crate) fn count_key(&mut self, key: StateKey) -> Result<(), Reason> {
        if self.keys.contains(&key) {
            return Ok(());
        }
        if self.keys.len() >= self.canon.budget.max_state_keys_mutated as usize {
            return Err(Reason::Budget);
        }
        self.keys.insert(key.clone());
        self.key_order.push(key);
        Ok(())
    }

    /// How many keys the round has counted so far.
    #[cfg(test)]
    pub(crate) fn keys_spent(&self) -> usize {
        self.key_order.len()
    }

    /// Give back every key counted after the first `from`.
    fn drop_keys(&mut self, from: usize) {
        for key in self.key_order.drain(from..) {
            self.keys.remove(&key);
        }
    }

    /// Every key whose value differs from what it was at the last take, sorted. Each record
    /// written since is compared once, from its first pre-image since the last take.
    pub(crate) fn take_changes(&mut self) -> Vec<Change> {
        let since = std::mem::replace(&mut self.frontier, self.pre_images.len());
        let mut diff = Diff::default();
        let mut compared: Vec<&PreImage> = Vec::new();
        for image in &self.pre_images[since..] {
            if compared.iter().any(|first| first.is_same_record(image)) {
                continue;
            }
            compared.push(image);
            image.diff_to_now(&self.canon, &self.state, &mut diff);
        }
        let mut changes = diff.changes;
        changes.sort_by(|left, right| left.key.cmp(&right.key));
        changes
    }

    pub(crate) fn count_memory_write(&mut self) -> Result<(), Reason> {
        if u64::from(self.memory_writes) >= self.canon.budget.max_memory_writes {
            return Err(Reason::Budget);
        }
        self.memory_writes += 1;
        Ok(())
    }

    pub(crate) fn push_memory(&mut self, line: MemoryLine) {
        self.memories.push(line);
    }

    /// Where the round has come to, to roll back to. It copies nothing.
    pub(crate) fn savepoint(&self) -> Savepoint {
        Savepoint {
            pre_images: self.pre_images.len(),
            keys: self.key_order.len(),
            memories: self.memories.len(),
            memory_writes: self.memory_writes,
        }
    }

    /// Undo everything since `point`: put back each record written since, newest first, and
    /// give back the keys and memories added since.
    pub(crate) fn rollback(&mut self, point: Savepoint) {
        for image in self.pre_images.drain(point.pre_images..).rev() {
            self.state.restore(image);
        }
        self.frontier = self.frontier.min(self.pre_images.len());
        self.drop_keys(point.keys);
        self.memories.truncate(point.memories);
        self.memory_writes = point.memory_writes;
    }
}

/// How far the round had come: the length of each log a rollback cuts back.
#[derive(Clone, Copy)]
pub(crate) struct Savepoint {
    pre_images: usize,
    keys: usize,
    memories: usize,
    memory_writes: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intent::Intent;
    use crate::model::key::StateValue;
    use crate::model::meter::MeterWrite;
    use crate::model::modifier::ModifierDef;
    use crate::rules::effect::{Effect, EffectKind, Holder};
    use crate::World;

    fn docks_world(adjust: impl FnOnce(&mut Canon)) -> World<()> {
        let pack =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packs/oneshot-docks");
        let opened = World::open(&pack).expect("open");
        let mut canon = opened.canon().clone();
        adjust(&mut canon);
        World::from_parts(&pack, Arc::new(canon), opened.state().clone())
    }

    fn apply(draft: &mut Draft, intent: &Intent, kind: EffectKind) -> Result<(), Reason> {
        Effect::new(intent.actor.as_ref(), &intent.id, kind).apply(draft, intent)
    }

    fn key_names(changes: &[Change]) -> Vec<String> {
        changes
            .iter()
            .map(|change| change.key.to_string())
            .collect()
    }

    #[test]
    fn a_key_counts_toward_the_budget_only_once_a_write_changes_it() {
        let world = docks_world(|canon| canon.budget.max_state_keys_mutated = 2);
        let nobody = crate::model::Presence::default();
        let mut draft = Draft::begin(&world, &nobody);
        // Bram's grit sits clamped at its minimum: taking more moves the offset, not the value.
        let meters = &mut draft.state.actor_mut("bram").unwrap().meters;
        meters.get_mut("grit").unwrap().offset = -5;
        let intent = Intent::habit(world.state(), "mira", "leave_door_note");
        let mut write = |actor: &str, key: &str, delta: i64| {
            let kind = EffectKind::Meter {
                holder: Holder::Actor(actor.to_string()),
                key: key.to_string(),
                write: MeterWrite::Offset(delta),
            };
            apply(&mut draft, &intent, kind)
        };
        assert_eq!(write("mira", "rest", 0), Ok(()), "unchanged: no key");
        assert_eq!(write("mira", "rest", 1), Ok(()), "the first key");
        assert_eq!(write("bram", "grit", -1), Ok(()), "offset moved: key two");
        assert_eq!(write("mira", "rest", 1), Ok(()), "a counted key is free");
        assert_eq!(write("aldric", "rest", 0), Ok(()), "unchanged: no key");
        assert_eq!(write("aldric", "rest", 1), Err(Reason::Budget));
    }

    #[test]
    fn a_modifier_spends_its_key_and_its_meters_key_and_the_meter_is_reported() {
        let world = docks_world(|canon| {
            canon.budget.max_state_keys_mutated = 2;
            canon.modifiers.insert(
                "burn".to_string(),
                ModifierDef {
                    meter: "grit".to_string(),
                    max_stacks: 2,
                    hidden: false,
                },
            );
        });
        let nobody = crate::model::Presence::default();
        let mut draft = Draft::begin(&world, &nobody);
        let intent = Intent::habit(world.state(), "mira", "leave_door_note");
        let burn = |add| EffectKind::Modifier {
            actor_id: "mira".to_string(),
            id: "burn".to_string(),
            add,
            ticks: 3,
        };
        let grit_before = world.state().actors["mira"].meters["grit"].clone();
        assert_eq!(apply(&mut draft, &intent, burn(2)), Ok(()));
        assert_eq!(
            draft.keys_spent(),
            2,
            "the modifier's key and the meter's, as on main"
        );
        assert_eq!(draft.state.actors["mira"].meters["grit"], grit_before);
        let changes = draft.take_changes();
        assert_eq!(
            key_names(&changes),
            ["actor_meter mira grit", "modifier mira burn"]
        );
        let (StateValue::Number(before), StateValue::Number(after)) =
            (&changes[0].before, &changes[0].after)
        else {
            panic!("{changes:?}");
        };
        assert_eq!(*after, before + 2);
        // A second stack moves the sum again, on the keys already spent.
        assert_eq!(apply(&mut draft, &intent, burn(2)), Ok(()));
        assert_eq!(draft.keys_spent(), 2);
        assert_eq!(draft.take_changes().len(), 2);
    }

    #[test]
    fn a_take_reports_the_net_change_since_the_last_take() {
        let world = docks_world(|_| {});
        let nobody = crate::model::Presence::default();
        let mut draft = Draft::begin(&world, &nobody);
        let intent = Intent::habit(world.state(), "mira", "leave_door_note");
        let rung = |value| EffectKind::Flag {
            holder: Holder::World,
            key: "rung".to_string(),
            value,
        };
        let edge = |changes: &[Change]| (changes[0].before.clone(), changes[0].after.clone());
        // Set and cleared before a take: no edge to report, but the key was spent.
        apply(&mut draft, &intent, rung(true)).unwrap();
        apply(&mut draft, &intent, rung(false)).unwrap();
        assert_eq!(draft.keys_spent(), 1);
        assert!(draft.take_changes().is_empty());
        // Set, taken, then cleared: two edges, one per take.
        apply(&mut draft, &intent, rung(true)).unwrap();
        let rose = draft.take_changes();
        assert_eq!(key_names(&rose), ["world_flag rung"]);
        assert_eq!(
            edge(&rose),
            (StateValue::Bool(false), StateValue::Bool(true))
        );
        apply(&mut draft, &intent, rung(false)).unwrap();
        let fell = draft.take_changes();
        assert_eq!(
            edge(&fell),
            (StateValue::Bool(true), StateValue::Bool(false))
        );
        // A write rolled back before the take is not reported, and its key is given back.
        let point = draft.savepoint();
        apply(&mut draft, &intent, rung(true)).unwrap();
        draft.rollback(point);
        assert!(draft.take_changes().is_empty());
        assert_eq!(draft.keys_spent(), 1);
    }
}
