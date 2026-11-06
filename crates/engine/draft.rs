//! The staged save a round writes before commit.

use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::Arc;

use crate::canon::Budget;
use crate::canon::Canon;
use crate::model::key::{Change, StateKey, StateValue};
use crate::model::ledger::{Ledger, Removed, Savepoint};
use crate::model::meter::Offset;
use crate::model::modifier::ModifierLive;
use crate::model::tracked::{Count, Member, Tracked};
use crate::model::{reading, ActCounter, ActorEdit, ComponentId, MemoryLine, Note, NoteEdit};
use crate::model::{PendingAction, PlaceEdit, Presence, State, StoryEdit};
use crate::rules::effect::Holder;
use crate::world::World;
use crate::Reason;

/// A round's staged save, with the canon it reads and who is seated. The writers below are the
/// only way a round changes the save: each lands at once, so a later read sees it, and goes on
/// the save's ledger, so it can be rolled back, counts toward the budget and is reported.
pub(crate) struct Draft<'p> {
    canon: Arc<Canon>,
    state: State,
    /// Who is seated for this round. The round reads it and never writes it.
    pub(crate) presence: &'p Presence,
    _nosend: PhantomData<Rc<()>>,
}

impl<'p> Draft<'p> {
    /// Stage a round on a copy of the world's save, with `presence` seated.
    pub(crate) fn begin<L>(world: &World<L>, presence: &'p Presence) -> Self {
        let mut state = world.state.clone();
        state.ledger = Ledger::new(world.canon.budget.max_state_keys_mutated as usize);
        Self {
            canon: Arc::clone(&world.canon),
            state,
            presence,
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

    /// The round's save and the memory lines it wrote. Its ledger still holds the round:
    /// commit persists the save, then rebases it.
    pub(crate) fn finish(mut self) -> (State, Vec<MemoryLine>) {
        let memories = self.state.ledger.take_memories();
        (self.state, memories)
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
        let due: Vec<(Holder, String)> = (self.state.modifiers.iter())
            .filter(|(_, _, held)| held.until <= world_time)
            .map(|(holder, id, _)| (holder.clone(), id.clone()))
            .collect();
        for (holder, id) in due {
            let Holder::Actor(actor) = holder else {
                continue;
            };
            self.put_modifier(&actor, &id, None)?;
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

    /// Make `edit` on actor `id`'s record. When the stored value moved, its key is counted
    /// toward the budget and its values before and after are kept for the next take; a value
    /// already in place counts nothing. A write the budget refuses stays in place for the
    /// caller's savepoint to undo.
    pub(crate) fn edit_actor(&mut self, id: &str, edit: ActorEdit) -> Result<(), Reason> {
        let State { actors, ledger, .. } = &mut self.state;
        let actor = actors.get_mut(id).ok_or(Reason::UnknownId)?;
        let at = || ComponentId::Actor(id.to_string());
        match edit.key(id) {
            Some(key) => ledger.land(at, actor, edit, key, |actor, key| actor.value(key)),
            None => {
                ledger.track(at, actor, edit);
                Ok(())
            }
        }
    }

    /// Make `edit` on place `id`'s live record, which it must have, counted and kept as
    /// `edit_actor` says.
    pub(crate) fn edit_place(&mut self, id: &str, edit: PlaceEdit) -> Result<(), Reason> {
        let State { places, ledger, .. } = &mut self.state;
        let place = places.get_mut(id).ok_or(Reason::UnknownId)?;
        let key = edit.key(id);
        let at = || ComponentId::Place(id.to_string());
        ledger.land(at, place, edit, key, |place, key| place.value(key))
    }

    /// Make `edit` on story `id`'s live record, which it must have, counted and kept as
    /// `edit_actor` says.
    pub(crate) fn edit_story(&mut self, id: &str, edit: StoryEdit) -> Result<(), Reason> {
        let State {
            stories, ledger, ..
        } = &mut self.state;
        let story = stories.get_mut(id).ok_or(Reason::UnknownId)?;
        let key = edit.key(id);
        let at = || ComponentId::Story(id.to_string());
        ledger.land(at, story, edit, key, |story, key| story.value(key))
    }

    /// Give place or story `holder` an empty live record when it has none. No key answers for
    /// it.
    pub(crate) fn make_record(&mut self, holder: &Holder) -> Result<(), Reason> {
        let state = &mut self.state;
        let at = match holder {
            Holder::Place(id) if !state.places.contains_key(id) => {
                state.places.insert(id.clone(), Tracked::default());
                ComponentId::Place(id.clone())
            }
            Holder::Story(id) if !state.stories.contains_key(id) => {
                state.stories.insert(id.clone(), Tracked::default());
                ComponentId::Story(id.clone())
            }
            Holder::Place(_) | Holder::Story(_) => return Ok(()),
            Holder::Actor(_) | Holder::Exit(..) | Holder::World => return Err(Reason::Schema),
        };
        state.ledger.added(at);
        Ok(())
    }

    /// Move meter `key` on `holder` by `edit`, counted and kept as `edit_actor` says, with the
    /// meter's value as `State::meter_value` reads it.
    pub(crate) fn edit_meter(
        &mut self,
        holder: &Holder,
        key: &str,
        edit: Offset,
    ) -> Result<(), Reason> {
        let Draft { canon, state, .. } = self;
        let watched = StateKey::meter(holder, key).ok_or(Reason::UnknownId)?;
        let before = reading(state.meter_value(canon, holder, key));
        let State { meters, ledger, .. } = &mut *state;
        let meter = meters.tracked_mut(holder, key).ok_or(Reason::UnknownId)?;
        let at = || ComponentId::Meter(holder.clone(), key.to_string());
        if !ledger.track(at, meter, edit) {
            return Ok(());
        }
        let after = reading(state.meter_value(canon, holder, key));
        state.ledger.note(watched, before, after)
    }

    /// Put modifier `id` on actor `actor_id` as `next`, `None` taking it off. Its stacks add to
    /// the meter its def names: when they move, that meter's key is counted first and its
    /// values kept too, and a modifier put on must leave the meter readable before its own
    /// key is counted. Otherwise as `edit_actor` says.
    pub(crate) fn put_modifier(
        &mut self,
        actor_id: &str,
        id: &str,
        next: Option<ModifierLive>,
    ) -> Result<(), Reason> {
        let Draft { canon, state, .. } = self;
        let canon: &Canon = canon;
        if !state.actors.contains_key(actor_id) {
            return Err(Reason::UnknownId);
        }
        let holder = Holder::Actor(actor_id.to_string());
        let meter = canon.modifiers.get(id).map(|def| def.meter.as_str());
        let meter_before = meter.map(|key| reading(state.meter_value(canon, &holder, key)));
        let held = state.modifiers.get(&holder, id);
        let stacks = held.map(|held| held.stacks.clone());
        let before = held.map_or(StateValue::Empty, ModifierLive::to_value);
        let puts = next.is_some();
        let at = || ComponentId::Modifier(holder.clone(), id.to_string());
        let State {
            modifiers, ledger, ..
        } = &mut *state;
        let moved = match (modifiers.tracked_mut(&holder, id), next) {
            (Some(held), Some(next)) => ledger.track(at, held, next),
            (Some(_), None) => {
                let held = modifiers.take(&holder, id).expect("a modifier held");
                ledger.removed(Removed::Modifier(holder.clone(), id.to_string(), held));
                true
            }
            (None, Some(next)) => {
                modifiers.put(holder.clone(), id.to_string(), Tracked::new(next));
                ledger.added(at());
                true
            }
            (None, None) => false,
        };
        if let (Some(meter), Some(before)) = (meter, meter_before) {
            let now = state.modifiers.get(&holder, id).map(|held| &held.stacks);
            let stacks_moved = now != stacks.as_ref();
            let key = StateKey::ActorMeter(actor_id.to_string(), meter.to_string());
            if stacks_moved {
                state.ledger.count_key(&key)?;
            }
            let after = state.meter_value(canon, &holder, meter);
            if puts {
                after?;
            }
            if stacks_moved {
                let after = reading(after);
                state.ledger.keep(Change { key, before, after });
            }
        }
        if moved {
            let after = (state.modifiers.get(&holder, id))
                .map_or(StateValue::Empty, ModifierLive::to_value);
            let key = StateKey::ActorModifier(actor_id.to_string(), id.to_string());
            state.ledger.note(key, before, after)?;
        }
        Ok(())
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
                let State { flags, ledger, .. } = &mut self.state;
                let key = StateKey::WorldFlag(key.to_string());
                let read = |flags: &Vec<String>, key: &StateKey| {
                    StateValue::Bool(
                        matches!(key, StateKey::WorldFlag(name) if flags.contains(name)),
                    )
                };
                ledger.land(|| ComponentId::WorldFlags, flags, edit, key, read)
            }
        }
    }

    /// Leave `note`, whose id no note has yet.
    pub(crate) fn leave_note(&mut self, note: Note) -> Result<(), Reason> {
        let State { notes, ledger, .. } = &mut self.state;
        let key = StateKey::Note(note.id.clone());
        let read = |notes: &Vec<Note>, key: &StateKey| match key {
            StateKey::Note(id) if notes.iter().any(|note| note.id == *id) => StateValue::Bool(true),
            _ => StateValue::Empty,
        };
        let edit = NoteEdit::Leave(note);
        ledger.land(|| ComponentId::Notes, notes, edit, key, read)
    }

    /// Count `key` toward the budget the first time this round, whether or not a write
    /// changed it: a cost of 0 still takes a key.
    pub(crate) fn count_key(&mut self, key: &StateKey) -> Result<(), Reason> {
        self.state.ledger.count_key(key)
    }

    /// How many keys the round has counted so far.
    #[cfg(test)]
    pub(crate) fn keys_spent(&self) -> usize {
        self.state.keys_spent()
    }

    /// Every key whose value differs from what it was at the last take, as
    /// `State::take_changes` says.
    pub(crate) fn take_changes(&mut self) -> Vec<Change> {
        self.state.take_changes()
    }

    pub(crate) fn count_memory_write(&mut self) -> Result<(), Reason> {
        let max = self.canon.budget.max_memory_writes;
        self.state.ledger.count_memory_write(max)
    }

    pub(crate) fn push_memory(&mut self, line: MemoryLine) {
        self.state.ledger.push_memory(line);
    }

    /// Where the round has come to, to roll back to.
    pub(crate) fn savepoint(&self) -> Savepoint {
        self.state.savepoint()
    }

    /// Undo everything since `point`, as `State::rollback` says.
    pub(crate) fn rollback(&mut self, point: Savepoint) {
        self.state.rollback(point);
    }
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
        let bram = Holder::Actor("bram".to_string());
        state.meters.frontier_mut(&bram, "grit").unwrap().offset = -5;
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
