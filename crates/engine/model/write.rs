//! Every write the save takes. In a round and out of one alike, each lands at once, so a later
//! read sees it, and goes on the save's ledger, so it can be rolled back, counts toward the
//! round's budget and is reported to the next take. No other code changes a component.

use crate::canon::Canon;
use crate::model::item::Shift;
use crate::model::key::{Change, StateKey, StateValue};
use crate::model::ledger::Removed;
use crate::model::meter::{MeterLive, Offset};
use crate::model::modifier::ModifierLive;
use crate::model::tracked::Tracked;
use crate::model::Note;
use crate::model::{number, reading, ActCounter, ActorEdit, ActorLive, ComponentId, MemoryLine};
use crate::model::{NoteEdit, PendingAction, State};
use crate::rules::effect::Holder;
use crate::Reason;

impl State {
    /// Set the clock the round plays at. Nothing rolls it back.
    pub(crate) fn stamp_round(&mut self, round_index: u64, world_time: u64, last_round_at: &str) {
        self.round_index = round_index;
        self.world_time = world_time;
        self.last_round_at = last_round_at.to_string();
    }

    /// Make `edit` on actor `id`'s record. When the stored value moved, its key is counted
    /// toward the budget and its values before and after are kept for the next take; a value
    /// already in place counts nothing. A write the budget refuses stays in place for the
    /// caller's savepoint to undo.
    pub fn edit_actor(&mut self, id: &str, edit: ActorEdit) -> Result<(), Reason> {
        let State { actors, ledger, .. } = self;
        let actor = actors.get_mut().get_mut(id).ok_or(Reason::UnknownId)?;
        let at = || ComponentId::Actor(id.to_string());
        match edit.key(id) {
            Some(key) => ledger.land(at, actor, edit, key, |actor, key| actor.value(key)),
            None => {
                ledger.track(at, actor, edit);
                Ok(())
            }
        }
    }

    /// Give place or story `holder` an empty live record when it has none. No key answers for
    /// it.
    pub fn make_record(&mut self, holder: &Holder) -> Result<(), Reason> {
        let at = match holder {
            Holder::Place(id) if !self.places.contains(id) => {
                self.places.get_mut().insert(id.clone());
                ComponentId::Place(id.clone())
            }
            Holder::Story(id) if !self.stories.contains(id) => {
                self.stories.get_mut().insert(id.clone());
                ComponentId::Story(id.clone())
            }
            Holder::Place(_) | Holder::Story(_) => return Ok(()),
            Holder::Actor(_) | Holder::Exit(..) | Holder::World => return Err(Reason::Schema),
        };
        self.ledger.added(at);
        Ok(())
    }

    /// Give the save actor `id`, with `record` and no components yet. Only fixtures make an
    /// actor: a pack's actors come from its save.
    pub fn add_actor(&mut self, id: &str, record: ActorLive) -> Result<(), Reason> {
        if self.actors.contains_key(id) {
            return Err(Reason::Schema);
        }
        self.actors.get_mut().insert(id.to_string(), record.into());
        self.ledger.added(ComponentId::Actor(id.to_string()));
        Ok(())
    }

    /// Take actor `id` out of the save: each of their components, then their record, each as
    /// one removal on the ledger, so a rollback puts them all back.
    pub fn remove_actor(&mut self, id: &str) -> Result<(), Reason> {
        let holder = Holder::Actor(id.to_string());
        let State {
            actors,
            items,
            meters,
            modifiers,
            flags,
            ledger,
            ..
        } = self;
        let record = actors.get_mut().remove(id).ok_or(Reason::UnknownId)?;
        for (key, part) in items.take_all(&holder) {
            ledger.removed(Removed::Item(holder.clone(), key, part));
        }
        for (key, part) in meters.take_all(&holder) {
            ledger.removed(Removed::Meter(holder.clone(), key, part));
        }
        for (key, part) in modifiers.take_all(&holder) {
            ledger.removed(Removed::Modifier(holder.clone(), key, part));
        }
        for (key, _) in flags.take_all(&holder) {
            ledger.removed(Removed::Flag(holder.clone(), key));
        }
        ledger.removed(Removed::Actor(id.to_string(), Box::new(record)));
        Ok(())
    }

    /// Put meter `key` on `holder` as `next`, `None` taking it off, outside the writes a
    /// round's rules make: migrate gives a holder the meters the pack adds and drops the ones
    /// it no longer gives. Counted and kept as `edit_meter` says.
    pub fn put_meter(
        &mut self,
        canon: &Canon,
        holder: &Holder,
        key: &str,
        next: Option<MeterLive>,
    ) -> Result<(), Reason> {
        let watched = StateKey::meter(holder, key).ok_or(Reason::Schema)?;
        if self.meters.get(holder, key) == next.as_ref() {
            return Ok(());
        }
        let before = reading(self.meter_value(canon, holder, key));
        let State { meters, ledger, .. } = &mut *self;
        if let Some(part) = meters.take(holder, key) {
            ledger.removed(Removed::Meter(holder.clone(), key.to_string(), part));
        }
        if let Some(live) = next {
            meters.put(holder.clone(), key.to_string(), Tracked::new(live));
            ledger.added(ComponentId::Meter(holder.clone(), key.to_string()));
        }
        let after = reading(self.meter_value(canon, holder, key));
        self.ledger.note(watched, before, after)
    }

    /// Move meter `key` on `holder` by `edit`, counted and kept as `edit_actor` says, with the
    /// meter's value as `State::meter_value` reads it.
    pub fn edit_meter(
        &mut self,
        canon: &Canon,
        holder: &Holder,
        key: &str,
        edit: Offset,
    ) -> Result<(), Reason> {
        let watched = StateKey::meter(holder, key).ok_or(Reason::UnknownId)?;
        let before = reading(self.meter_value(canon, holder, key));
        let State { meters, ledger, .. } = &mut *self;
        let meter = meters.tracked_mut(holder, key).ok_or(Reason::UnknownId)?;
        let at = || ComponentId::Meter(holder.clone(), key.to_string());
        if !ledger.track(at, meter, edit) {
            return Ok(());
        }
        let after = reading(self.meter_value(canon, holder, key));
        self.ledger.note(watched, before, after)
    }

    /// Put modifier `id` on actor `actor_id` as `next`, `None` taking it off. Its stacks add to
    /// the meter its def names: when they move, that meter's key is counted first and its
    /// values kept too, and a modifier put on must leave the meter readable before its own
    /// key is counted. Otherwise as `edit_actor` says.
    pub fn put_modifier(
        &mut self,
        canon: &Canon,
        actor_id: &str,
        id: &str,
        next: Option<ModifierLive>,
    ) -> Result<(), Reason> {
        if !self.actors.contains_key(actor_id) {
            return Err(Reason::UnknownId);
        }
        let holder = Holder::Actor(actor_id.to_string());
        let meter = canon.modifiers.get(id).map(|def| def.meter.as_str());
        let meter_before = meter.map(|key| reading(self.meter_value(canon, &holder, key)));
        let held = self.modifiers.get(&holder, id);
        let stacks = held.map(|held| held.stacks.clone());
        let before = held.map_or(StateValue::Empty, ModifierLive::to_value);
        let puts = next.is_some();
        let at = || ComponentId::Modifier(holder.clone(), id.to_string());
        let State {
            modifiers, ledger, ..
        } = &mut *self;
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
            let now = self.modifiers.get(&holder, id).map(|held| &held.stacks);
            let stacks_moved = now != stacks.as_ref();
            let key = StateKey::ActorMeter(actor_id.to_string(), meter.to_string());
            if stacks_moved {
                self.ledger.count_key(&key)?;
            }
            let after = self.meter_value(canon, &holder, meter);
            if puts {
                after?;
            }
            if stacks_moved {
                let after = reading(after);
                self.ledger.keep(Change { key, before, after });
            }
        }
        if moved {
            let after =
                (self.modifiers.get(&holder, id)).map_or(StateValue::Empty, ModifierLive::to_value);
            let key = StateKey::ActorModifier(actor_id.to_string(), id.to_string());
            self.ledger.note(key, before, after)?;
        }
        Ok(())
    }

    /// Expire every modifier whose tick has come by `world_time`, all its stacks at once.
    pub(crate) fn expire_modifiers(
        &mut self,
        canon: &Canon,
        world_time: u64,
    ) -> Result<(), Reason> {
        let due: Vec<(Holder, String)> = (self.modifiers.iter())
            .filter(|(_, _, held)| held.until <= world_time)
            .map(|(holder, id, _)| (holder.clone(), id.clone()))
            .collect();
        for (holder, id) in due {
            let Holder::Actor(actor) = holder else {
                continue;
            };
            self.put_modifier(canon, &actor, &id, None)?;
        }
        Ok(())
    }

    /// Set how many of item `id` `holder` holds to `count`, counted and kept as `edit_actor`
    /// says. Only an actor or a place holds items, and it must be in the save.
    pub fn set_items(&mut self, holder: &Holder, id: &str, count: u32) -> Result<(), Reason> {
        let watched = StateKey::item(holder, id).ok_or(Reason::Schema)?;
        let held = self.item_count(holder, id).ok_or(Reason::UnknownId)?;
        if held == count {
            return Ok(());
        }
        let State { items, ledger, .. } = self;
        let at = || ComponentId::Item(holder.clone(), id.to_string());
        if held == 0 {
            items.put(holder.clone(), id.to_string(), Tracked::new(count));
            ledger.added(at());
        } else if count == 0 {
            let part = items.take(holder, id).expect("a count above 0");
            ledger.removed(Removed::Item(holder.clone(), id.to_string(), part));
        } else {
            let part = items.tracked_mut(holder, id).expect("a count above 0");
            ledger.track(at, part, Shift(i64::from(count) - i64::from(held)));
        }
        ledger.note(watched, number(u64::from(held)), number(u64::from(count)))
    }

    /// Set `holder`'s flag `key` on or off, counted and kept as `edit_actor` says. An exit's
    /// one flag is `open`. A flag already so changes nothing.
    pub fn set_flag(&mut self, holder: &Holder, key: &str, on: bool) -> Result<(), Reason> {
        let State { flags, ledger, .. } = self;
        if flags.get(holder, key).is_some() == on {
            return Ok(());
        }
        let (holder, key) = (holder.clone(), key.to_string());
        let watched = StateKey::flag(&holder, &key);
        if on {
            flags.put(holder.clone(), key.clone(), Tracked::default());
            ledger.added(ComponentId::Flag(holder, key));
        } else {
            flags.take(&holder, &key);
            ledger.removed(Removed::Flag(holder, key));
        }
        ledger.note(watched, StateValue::Bool(!on), StateValue::Bool(on))
    }

    /// Leave `note`, whose id no note has yet.
    pub fn leave_note(&mut self, note: Note) -> Result<(), Reason> {
        let State { notes, ledger, .. } = self;
        let key = StateKey::Note(note.id.clone());
        let read = |notes: &Vec<Note>, key: &StateKey| match key {
            StateKey::Note(id) if notes.iter().any(|note| note.id == *id) => StateValue::Bool(true),
            _ => StateValue::Empty,
        };
        let edit = NoteEdit::Leave(note);
        ledger.land(|| ComponentId::Notes, notes, edit, key, read)
    }

    /// Set `counter` back to 0 on every actor.
    pub(crate) fn reset_acts(&mut self, counter: ActCounter) -> Result<(), Reason> {
        let ids: Vec<String> = self.actors.keys().cloned().collect();
        for id in ids {
            self.edit_actor(&id, ActorEdit::Acts(counter, 0))?;
        }
        Ok(())
    }

    /// Count one act for `actor_id` this round, in the act window the table is in.
    pub(crate) fn count_act(&mut self, actor_id: &str) -> Result<(), Reason> {
        let round = self.round_index;
        let act = &self.actors.get(actor_id).ok_or(Reason::UnknownId)?.act;
        let (counter, count) = if self.presence().players == 0 {
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
        let actor = self.actors.get(id).ok_or(Reason::UnknownId)?;
        let taken = actor.act.pending.clone();
        self.edit_actor(id, ActorEdit::Busy(None, None))?;
        Ok(taken)
    }

    /// Count `key` toward the budget the first time this round, whether or not a write
    /// changed it: a cost of 0 still takes a key.
    pub(crate) fn count_key(&mut self, key: &StateKey) -> Result<(), Reason> {
        self.ledger.count_key(key)
    }

    /// Count one memory write toward the round's limit.
    pub(crate) fn count_memory_write(&mut self) -> Result<(), Reason> {
        self.ledger.count_memory_write()
    }

    /// Keep `line` for the round's journal record.
    pub(crate) fn push_memory(&mut self, line: MemoryLine) {
        self.ledger.push_memory(line);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

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
        let clamped = MeterLive {
            base: 1,
            offset: -5,
        };
        state
            .put_meter(&canon, &bram, "grit", Some(clamped))
            .unwrap();
        state.accept();
        let world = World::from_parts(&pack, Arc::new(canon), state);
        let nobody = crate::model::Presence::default();
        let mut state = world.begin_round(&nobody);
        let intent = Intent::habit(world.state(), "mira", "leave_door_note");
        let mut write = |actor: &str, key: &str, delta: i64| {
            let kind = EffectKind::Meter {
                holder: Holder::Actor(actor.to_string()),
                key: key.to_string(),
                write: MeterWrite::Offset(delta),
            };
            let effect = Effect::new(intent.actor.as_ref(), &intent.id, kind);
            effect.apply(world.canon(), &mut state, &intent)
        };
        assert_eq!(write("mira", "rest", 0), Ok(()), "unchanged: no key");
        assert_eq!(write("mira", "rest", 1), Ok(()), "the first key");
        assert_eq!(write("bram", "grit", -1), Ok(()), "offset moved: key two");
        assert_eq!(write("mira", "rest", 1), Ok(()), "a counted key is free");
        assert_eq!(write("aldric", "rest", 0), Ok(()), "unchanged: no key");
        assert_eq!(write("aldric", "rest", 1), Err(Reason::Budget));
    }
}
