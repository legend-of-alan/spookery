//! The staged save a round writes before commit.

use std::collections::{BTreeMap, BTreeSet};
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::Arc;

use crate::canon::Budget;
use crate::canon::Canon;
use crate::model::key::{Change, StateKey, StateValue};
use crate::model::meter::Offset;
use crate::model::tracked::{Count, Edit, Entry, Member, Tracked};
use crate::model::{self, reading, ActCounter, ActorEdit, MemoryLine, Note, NoteEdit};
use crate::model::{PendingAction, PlaceEdit, Presence, RecordId, State, StoryEdit};
use crate::rules::effect::Holder;
use crate::world::World;
use crate::Reason;

pub(crate) struct Draft<'p> {
    canon: Arc<Canon>,
    /// Only the edits below change it, so every change can be rolled back.
    state: State,
    /// Who is seated for this round. The round reads it and never writes it.
    pub(crate) presence: &'p Presence,
    ledger: Ledger,
    memories: Vec<MemoryLine>,
    memory_writes: u32,
    _nosend: PhantomData<Rc<()>>,
}

/// What the round's writes owe: the keys counted toward the budget, each change for the next
/// take, and each record they touched, for a rollback.
struct Ledger {
    max_keys: usize,
    /// The keys counted toward the budget this round.
    keys: BTreeSet<StateKey>,
    /// The same keys in the order they were counted, so a savepoint can give them back.
    key_order: Vec<StateKey>,
    /// Each record a write changed or brought into being, oldest first.
    touched: Vec<Touch>,
    /// Each key a write moved, with its value before and after that write, oldest first.
    changes: Vec<Change>,
    /// How many changes the last take had reported.
    taken: usize,
}

/// A record a write changed, with how many edits it held before, or one a write brought into
/// being. A rollback undoes each touch since its savepoint, newest first.
enum Touch {
    Edited(RecordId, usize),
    NewPlace(String),
    NewStory(String),
}

impl<'p> Draft<'p> {
    /// Stage a round on a copy of the world's save, with `presence` seated.
    pub(crate) fn begin<L>(world: &World<L>, presence: &'p Presence) -> Self {
        Self {
            canon: Arc::clone(&world.canon),
            state: world.state.clone(),
            presence,
            ledger: Ledger {
                max_keys: world.canon.budget.max_state_keys_mutated as usize,
                keys: BTreeSet::new(),
                key_order: Vec::new(),
                touched: Vec::new(),
                changes: Vec::new(),
                taken: 0,
            },
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

    /// The round's save, as the frontier the next round starts from: nothing it wrote can be
    /// rolled back now.
    pub(crate) fn finish(mut self) -> (State, Vec<MemoryLine>) {
        for touch in self.ledger.touched.drain(..) {
            if let Touch::Edited(at, _) = touch {
                if let Some(record) = self.state.record_mut(&at) {
                    record.rebase(0);
                }
            }
        }
        (self.state, self.memories)
    }

    /// Set `counter` back to 0 on every actor.
    pub(crate) fn reset_acts(&mut self, counter: ActCounter) -> Result<(), Reason> {
        let ids: Vec<String> = self.state.actors.keys().cloned().collect();
        for id in ids {
            self.edit_actor(&id, ActorEdit::Acts(counter, 0))?;
        }
        Ok(())
    }

    /// Expire every modifier whose tick has come by `world_time`, all its stacks at once.
    pub(crate) fn expire_modifiers(&mut self, world_time: u64) -> Result<(), Reason> {
        let due: Vec<(String, String)> = self
            .state
            .actors
            .iter()
            .flat_map(|(actor, live)| {
                live.modifiers
                    .iter()
                    .filter(|(_, held)| held.until <= world_time)
                    .map(|(id, _)| (actor.clone(), id.clone()))
            })
            .collect();
        for (actor, id) in due {
            self.edit_actor(&actor, ActorEdit::Modifier(Entry { id, value: None }))?;
        }
        Ok(())
    }

    /// Count one act for `actor_id` this round, in the act window the table is in.
    pub(crate) fn count_act(&mut self, actor_id: &str) -> Result<(), Reason> {
        let round = self.state.round_index;
        let act = &self
            .state
            .actors
            .get(actor_id)
            .ok_or(Reason::UnknownId)?
            .act;
        let (counter, count) = if self.presence.players == 0 {
            (ActCounter::IdleActsThisWindow, act.idle_acts_this_window)
        } else {
            (ActCounter::LiveActsThisWindow, act.live_acts_this_window)
        };
        self.edit_actor(actor_id, ActorEdit::LastActRound(round))?;
        self.edit_actor(actor_id, ActorEdit::Acts(counter, count.saturating_add(1)))
    }

    /// Keep actor `id` busy until `until`, on `pending` when it is an action under way and
    /// `None` when it is a cooldown.
    pub(crate) fn set_busy(
        &mut self,
        id: &str,
        until: u64,
        pending: Option<PendingAction>,
    ) -> Result<(), Reason> {
        self.edit_actor(id, ActorEdit::Busy(Some(until), pending))
    }

    /// End actor `id`'s action under way, if any, and free them: it never finishes here.
    pub(crate) fn take_pending(&mut self, id: &str) -> Result<Option<PendingAction>, Reason> {
        let actor = self.state.actors.get(id).ok_or(Reason::UnknownId)?;
        let taken = actor.act.pending.clone();
        self.edit_actor(id, ActorEdit::Busy(None, None))?;
        Ok(taken)
    }

    /// Make `edit` on actor `id`: the one way a round changes an actor, so a later read, in the
    /// same effect or the next, sees it. When the stored value moved, its key is counted toward
    /// the budget and its values before and after are kept for the next take; a value already
    /// in place counts nothing. A modifier's stacks add to the meter its def names: when they
    /// move, that meter's key is counted first and its values kept too, and a modifier put in
    /// place must leave the meter readable before its own key is counted. A write the budget
    /// refuses stays in place for the caller's savepoint to undo.
    pub(crate) fn edit_actor(&mut self, id: &str, edit: ActorEdit) -> Result<(), Reason> {
        let Draft {
            canon,
            state,
            ledger,
            ..
        } = self;
        let canon: &Canon = canon;
        let actor = state.actors.get_mut(id).ok_or(Reason::UnknownId)?;
        let added_to = match &edit {
            ActorEdit::Modifier(entry) => canon.modifiers.get(&entry.id).map(|def| {
                let stacks = actor
                    .modifiers
                    .get(&entry.id)
                    .map(|held| held.stacks.clone());
                let before = reading(actor.meter_value(canon, id, &def.meter));
                let puts = entry.value.is_some();
                (entry.id.clone(), puts, def.meter.as_str(), stacks, before)
            }),
            _ => None,
        };
        let key = edit.key(id);
        let before = key.as_ref().map(|key| actor.value(canon, id, key));
        let moved = ledger.track(|| RecordId::Actor(id.to_string()), actor, edit);
        if let Some((modifier, puts, meter, stacks, before)) = added_to {
            let stacks_moved =
                actor.modifiers.get(&modifier).map(|held| &held.stacks) != stacks.as_ref();
            let key = StateKey::ActorMeter(id.to_string(), meter.to_string());
            if stacks_moved {
                ledger.count_key(&key)?;
            }
            let after = actor.meter_value(canon, id, meter);
            if puts {
                after?;
            }
            if stacks_moved {
                let after = reading(after);
                ledger.changes.push(Change { key, before, after });
            }
        }
        if let (true, Some(key), Some(before)) = (moved, key, before) {
            let after = actor.value(canon, id, &key);
            ledger.note(key, before, after)?;
        }
        Ok(())
    }

    /// Make `edit` on place `id`'s live record, which it must have, counted and kept as
    /// `edit_actor` says.
    pub(crate) fn edit_place(&mut self, id: &str, edit: PlaceEdit) -> Result<(), Reason> {
        let Draft {
            canon,
            state,
            ledger,
            ..
        } = self;
        let place = state.places.get_mut(id).ok_or(Reason::UnknownId)?;
        let key = edit.key(id);
        let at = || RecordId::Place(id.to_string());
        ledger.land(at, place, edit, key, |place, key| {
            place.value(canon, id, key)
        })
    }

    /// Make `edit` on story `id`'s live record, which it must have, counted and kept as
    /// `edit_actor` says.
    pub(crate) fn edit_story(&mut self, id: &str, edit: StoryEdit) -> Result<(), Reason> {
        let Draft {
            canon,
            state,
            ledger,
            ..
        } = self;
        let story = state.stories.get_mut(id).ok_or(Reason::UnknownId)?;
        let key = edit.key(id);
        let at = || RecordId::Story(id.to_string());
        ledger.land(at, story, edit, key, |story, key| {
            story.value(canon, id, key)
        })
    }

    /// Give place or story `holder` an empty live record when it has none. No key answers for
    /// it.
    pub(crate) fn make_record(&mut self, holder: &Holder) -> Result<(), Reason> {
        let touch = match holder {
            Holder::Place(id) if !self.state.places.contains_key(id) => {
                self.state.places.insert(id.clone(), Tracked::default());
                Touch::NewPlace(id.clone())
            }
            Holder::Story(id) if !self.state.stories.contains_key(id) => {
                self.state.stories.insert(id.clone(), Tracked::default());
                Touch::NewStory(id.clone())
            }
            Holder::Place(_) | Holder::Story(_) => return Ok(()),
            Holder::Actor(_) | Holder::Exit(..) | Holder::World => return Err(Reason::Schema),
        };
        self.ledger.touched.push(touch);
        Ok(())
    }

    /// Make `edit` on the meters `holder` carries. An exit carries none.
    pub(crate) fn edit_meters(&mut self, holder: &Holder, edit: Offset) -> Result<(), Reason> {
        match holder {
            Holder::Actor(id) => self.edit_actor(id, ActorEdit::Meter(edit)),
            Holder::Place(id) => self.edit_place(id, PlaceEdit::Meter(edit)),
            Holder::Story(id) => self.edit_story(id, StoryEdit::Meter(edit)),
            Holder::Exit(..) => Err(Reason::UnknownId),
            Holder::World => {
                let Draft {
                    canon,
                    state,
                    ledger,
                    ..
                } = self;
                let name = edit.key.clone();
                let key = StateKey::WorldMeter(edit.key.clone());
                let read = |meters: &BTreeMap<_, _>, _: &StateKey| {
                    reading(model::meter_value(canon, &Holder::World, meters, &name, 0))
                };
                let meters = &mut state.world_meters;
                ledger.land(|| RecordId::WorldMeters, meters, edit, key, read)
            }
        }
    }

    /// Make `edit` on the item counts `holder` holds. Only an actor or a place holds items.
    pub(crate) fn edit_items(&mut self, holder: &Holder, edit: Count) -> Result<(), Reason> {
        match holder {
            Holder::Actor(id) => self.edit_actor(id, ActorEdit::Item(edit)),
            Holder::Place(id) => self.edit_place(id, PlaceEdit::Item(edit)),
            Holder::Story(_) | Holder::Exit(..) | Holder::World => Err(Reason::Schema),
        }
    }

    /// Set `holder`'s flag `key` on or off. An exit's one flag is `open`, its name in its
    /// place's list. A flag already so changes nothing, so it needs no live record.
    pub(crate) fn set_flag(&mut self, holder: &Holder, key: &str, on: bool) -> Result<(), Reason> {
        if self.state.has_flag(holder, key) == on {
            return Ok(());
        }
        let edit = Member::set(key, on);
        match holder {
            Holder::Actor(id) => self.edit_actor(id, ActorEdit::Flag(edit)),
            Holder::Place(id) => self.edit_place(id, PlaceEdit::Flag(edit)),
            Holder::Story(id) => self.edit_story(id, StoryEdit::Flag(edit)),
            Holder::Exit(place, exit) => {
                self.edit_place(place, PlaceEdit::Open(Member::set(exit, on)))
            }
            Holder::World => {
                let Draft { state, ledger, .. } = self;
                let key = StateKey::WorldFlag(key.to_string());
                let read = |flags: &Vec<String>, key: &StateKey| {
                    StateValue::Bool(
                        matches!(key, StateKey::WorldFlag(name) if flags.contains(name)),
                    )
                };
                ledger.land(|| RecordId::WorldFlags, &mut state.flags, edit, key, read)
            }
        }
    }

    /// Leave `note`, whose id no note has yet.
    pub(crate) fn leave_note(&mut self, note: Note) -> Result<(), Reason> {
        let Draft { state, ledger, .. } = self;
        let key = StateKey::Note(note.id.clone());
        let read = |notes: &Vec<Note>, key: &StateKey| match key {
            StateKey::Note(id) if notes.iter().any(|note| note.id == *id) => StateValue::Bool(true),
            _ => StateValue::Empty,
        };
        let edit = NoteEdit::Leave(note);
        ledger.land(|| RecordId::Notes, &mut state.notes, edit, key, read)
    }

    /// Count `key` toward the budget the first time this round, whether or not a write
    /// changed it: a cost of 0 still takes a key.
    pub(crate) fn count_key(&mut self, key: &StateKey) -> Result<(), Reason> {
        self.ledger.count_key(key)
    }

    /// How many keys the round has counted so far.
    #[cfg(test)]
    pub(crate) fn keys_spent(&self) -> usize {
        self.ledger.key_order.len()
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
            touched: self.ledger.touched.len(),
            keys: self.ledger.key_order.len(),
            changes: self.ledger.changes.len(),
            taken: self.ledger.taken,
            memories: self.memories.len(),
            memory_writes: self.memory_writes,
        }
    }

    /// Undo everything since `point`: roll back each record touched since, newest first, and
    /// give back the keys, changes and memories added since.
    pub(crate) fn rollback(&mut self, point: Savepoint) {
        let ledger = &mut self.ledger;
        for touch in ledger.touched.drain(point.touched..).rev() {
            match touch {
                Touch::Edited(at, edits) => {
                    if let Some(record) = self.state.record_mut(&at) {
                        record.rollback(edits);
                    }
                }
                Touch::NewPlace(id) => {
                    self.state.places.remove(&id);
                }
                Touch::NewStory(id) => {
                    self.state.stories.remove(&id);
                }
            }
        }
        for key in ledger.key_order.drain(point.keys..) {
            ledger.keys.remove(&key);
        }
        ledger.changes.truncate(point.changes);
        ledger.taken = point.taken;
        self.memories.truncate(point.memories);
        self.memory_writes = point.memory_writes;
    }
}

impl Ledger {
    /// Make `edit` on `record`, which `at` names, and keep how many edits it held before, for a
    /// rollback. Whether it moved.
    fn track<T, E: Edit<T>>(
        &mut self,
        at: impl FnOnce() -> RecordId,
        record: &mut Tracked<T, E>,
        edit: E,
    ) -> bool {
        let edits = record.edits();
        let moved = record.apply(edit);
        if moved {
            self.touched.push(Touch::Edited(at(), edits));
        }
        moved
    }

    /// Make `edit` on `record`, and when it moved, count `key` and keep its values before and
    /// after, as `read` reads them.
    fn land<T, E: Edit<T>>(
        &mut self,
        at: impl FnOnce() -> RecordId,
        record: &mut Tracked<T, E>,
        edit: E,
        key: StateKey,
        read: impl Fn(&T, &StateKey) -> StateValue,
    ) -> Result<(), Reason> {
        let before = read(record, &key);
        if !self.track(at, record, edit) {
            return Ok(());
        }
        let after = read(record, &key);
        self.note(key, before, after)
    }

    /// Count `key` and keep its change for the next take.
    fn note(&mut self, key: StateKey, before: StateValue, after: StateValue) -> Result<(), Reason> {
        self.count_key(&key)?;
        self.changes.push(Change { key, before, after });
        Ok(())
    }

    fn count_key(&mut self, key: &StateKey) -> Result<(), Reason> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intent::Intent;
    use crate::model::meter::MeterWrite;
    use crate::rules::effect::{Effect, EffectKind};
    use crate::World;

    #[test]
    fn a_key_counts_toward_the_budget_only_once_a_write_changes_it() {
        let pack =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packs/oneshot-docks");
        let opened = World::open(&pack).expect("open");
        let mut canon = opened.canon().clone();
        canon.budget.max_state_keys_mutated = 2;
        // Bram's grit sits clamped at its minimum: taking more moves the offset, not the value.
        let mut state = opened.state().clone();
        let bram = state.actors.get_mut("bram").unwrap().frontier_mut();
        bram.meters.get_mut("grit").unwrap().offset = -5;
        let world = World::from_parts(&pack, Arc::new(canon), state);
        let nobody = crate::model::Presence::default();
        let mut draft = Draft::begin(&world, &nobody);
        let intent = Intent::habit(world.state(), "mira", "leave_door_note");
        let mut write = |actor: &str, key: &str, delta: i64| {
            let kind = EffectKind::Meter {
                holder: Holder::Actor(actor.to_string()),
                key: key.to_string(),
                write: MeterWrite::Offset(delta),
            };
            Effect::new(intent.actor.as_ref(), &intent.id, kind).apply(&mut draft, &intent)
        };
        assert_eq!(write("mira", "rest", 0), Ok(()), "unchanged: no key");
        assert_eq!(write("mira", "rest", 1), Ok(()), "the first key");
        assert_eq!(write("bram", "grit", -1), Ok(()), "offset moved: key two");
        assert_eq!(write("mira", "rest", 1), Ok(()), "a counted key is free");
        assert_eq!(write("aldric", "rest", 0), Ok(()), "unchanged: no key");
        assert_eq!(write("aldric", "rest", 1), Err(Reason::Budget));
    }
}
