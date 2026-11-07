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
use crate::model::MemoryLine;
use crate::model::Note;
use crate::model::{number, reading, ActCounter, ActorEdit, ComponentId, ExitId};
use crate::model::{NoteEdit, PendingAction, State};
use crate::rules::effect::EntityId;
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
        let actor = actors.records.tracked_mut(id).ok_or(Reason::UnknownId)?;
        let at = || ComponentId::Actor(id.to_string());
        match edit.key(id) {
            Some(key) => ledger.land(at, actor, edit, key, |actor, key| actor.value(key)),
            None => {
                ledger.track(at, actor, edit);
                Ok(())
            }
        }
    }

    /// Put meter `key` on `entity` as `next`, `None` taking it off, outside the writes a
    /// round's rules make: migrate gives an entity the meters the pack adds and drops the ones
    /// it no longer gives. Counted and kept as `edit_meter` says.
    pub fn put_meter(
        &mut self,
        canon: &Canon,
        entity: &EntityId,
        key: &str,
        next: Option<MeterLive>,
    ) -> Result<(), Reason> {
        if self.meters_of(entity).get(key) == next.as_ref() {
            return Ok(());
        }
        if next.is_some() {
            self.require_entity(entity)?;
        }
        let before = reading(self.meter_value(canon, entity, key));
        let (mut meters, watched, ledger) = self.meters_of_mut(entity, key);
        if let Some(part) = meters.take(key) {
            ledger.removed(Removed::Meter(entity.clone(), key.to_string(), part));
        }
        if let Some(live) = next {
            meters.put(key.to_string(), Tracked::new(live));
            ledger.added(ComponentId::Meter(entity.clone(), key.to_string()));
        }
        let after = reading(self.meter_value(canon, entity, key));
        self.ledger.note(watched, before, after)
    }

    /// Move meter `key` on `entity` by `edit`, counted and kept as `edit_actor` says, with the
    /// meter's value as `State::meter_value` reads it.
    pub fn edit_meter(
        &mut self,
        canon: &Canon,
        entity: &EntityId,
        key: &str,
        edit: Offset,
    ) -> Result<(), Reason> {
        let before = reading(self.meter_value(canon, entity, key));
        let (meters, watched, ledger) = self.meters_of_mut(entity, key);
        let meter = meters.tracked_mut(key).ok_or(Reason::UnknownId)?;
        let at = || ComponentId::Meter(entity.clone(), key.to_string());
        if !ledger.track(at, meter, edit) {
            return Ok(());
        }
        let after = reading(self.meter_value(canon, entity, key));
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
        let entity = EntityId::Actor(actor_id.to_string());
        self.require_entity(&entity)?;
        let meter = canon.modifiers.get(id).map(|def| def.meter.as_str());
        let meter_before = meter.map(|key| reading(self.meter_value(canon, &entity, key)));
        let held = self.actors.modifiers.get(actor_id, id);
        let stacks = held.map(|held| held.stacks.clone());
        let before = held.map_or(StateValue::Empty, ModifierLive::to_value);
        let puts = next.is_some();
        let at = || ComponentId::Modifier(actor_id.to_string(), id.to_string());
        let State { actors, ledger, .. } = &mut *self;
        let modifiers = &mut actors.modifiers;
        let moved = match (modifiers.tracked_mut(actor_id, id), next) {
            (Some(held), Some(next)) => ledger.track(at, held, next),
            (Some(_), None) => {
                let held = modifiers.take(actor_id, id).expect("a modifier held");
                ledger.removed(Removed::Modifier(
                    actor_id.to_string(),
                    id.to_string(),
                    held,
                ));
                true
            }
            (None, Some(next)) => {
                modifiers.put(actor_id, id.to_string(), Tracked::new(next));
                ledger.added(at());
                true
            }
            (None, None) => false,
        };
        if let (Some(meter), Some(before)) = (meter, meter_before) {
            let now = self
                .actors
                .modifiers
                .get(actor_id, id)
                .map(|held| &held.stacks);
            let stacks_moved = now != stacks.as_ref();
            let key = StateKey::ActorMeter(actor_id.to_string(), meter.to_string());
            if stacks_moved {
                self.ledger.count_key(&key)?;
            }
            let after = self.meter_value(canon, &entity, meter);
            if puts {
                after?;
            }
            if stacks_moved {
                let after = reading(after);
                self.ledger.keep(Change { key, before, after });
            }
        }
        if moved {
            let after = (self.actors.modifiers.get(actor_id, id))
                .map_or(StateValue::Empty, ModifierLive::to_value);
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
        let due: Vec<(String, String)> = (self.actors.modifiers.iter())
            .filter(|(_, _, held)| held.until <= world_time)
            .map(|(actor, id, _)| (actor.clone(), id.clone()))
            .collect();
        for (actor, id) in due {
            self.put_modifier(canon, &actor, &id, None)?;
        }
        Ok(())
    }

    /// Set how many of item `id` `entity` holds to `count`, counted and kept as `edit_actor`
    /// says. Only an actor or a place holds items, and it must be in the save.
    pub fn set_items(&mut self, entity: &EntityId, id: &str, count: u32) -> Result<(), Reason> {
        let held = self.item_count(entity, id);
        let (mut items, watched, ledger) = self.items_of_mut(entity, id).ok_or(Reason::Schema)?;
        let held = held.ok_or(Reason::UnknownId)?;
        if held == count {
            return Ok(());
        }
        let at = || ComponentId::Item(entity.clone(), id.to_string());
        if held == 0 {
            items.put(id.to_string(), Tracked::new(count));
            ledger.added(at());
        } else if count == 0 {
            let part = items.take(id).expect("a count above 0");
            ledger.removed(Removed::Item(entity.clone(), id.to_string(), part));
        } else {
            let part = items.tracked_mut(id).expect("a count above 0");
            ledger.track(at, part, Shift(i64::from(count) - i64::from(held)));
        }
        ledger.note(watched, number(u64::from(held)), number(u64::from(count)))
    }

    /// Set `entity`'s flag `key` on or off, counted and kept as `edit_actor` says. A flag
    /// already so changes nothing.
    pub fn set_flag(&mut self, entity: &EntityId, key: &str, on: bool) -> Result<(), Reason> {
        if self.has_flag(entity, key) == on {
            return Ok(());
        }
        if on {
            self.require_entity(entity)?;
        }
        let (mut flags, watched, ledger) = self.flags_of_mut(entity, key);
        if on {
            flags.put(key.to_string(), Tracked::default());
            ledger.added(ComponentId::Flag(entity.clone(), key.to_string()));
        } else {
            flags.take(key);
            ledger.removed(Removed::Flag(entity.clone(), key.to_string()));
        }
        ledger.note(watched, StateValue::Bool(!on), StateValue::Bool(on))
    }

    /// Open or shut the door of `exit`, counted and kept as `edit_actor` says. A door already
    /// so changes nothing.
    pub fn set_exit_open(&mut self, exit: &ExitId, open: bool) -> Result<(), Reason> {
        if self.is_exit_open(exit) == open {
            return Ok(());
        }
        self.put_exit_open(exit, open);
        if open {
            self.ledger.added(ComponentId::ExitOpen(exit.clone()));
        } else {
            self.ledger.removed(Removed::ExitOpen(exit.clone()));
        }
        let watched = StateKey::ExitOpen(exit.place.clone(), exit.exit.clone());
        self.ledger
            .note(watched, StateValue::Bool(!open), StateValue::Bool(open))
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
        let ids: Vec<String> = self.actors.records.keys().cloned().collect();
        for id in ids {
            self.edit_actor(&id, ActorEdit::Acts(counter, 0))?;
        }
        Ok(())
    }

    /// Count one act for `actor_id` this round, in the act window the table is in.
    pub(crate) fn count_act(&mut self, actor_id: &str) -> Result<(), Reason> {
        let round = self.round_index;
        let act = &self
            .actors
            .records
            .get(actor_id)
            .ok_or(Reason::UnknownId)?
            .act;
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
        let actor = self.actors.records.get(id).ok_or(Reason::UnknownId)?;
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

/// Making, copying and removing an actor, with every component they hold. Only fixtures do: a
/// pack's actors come from its save, and none leaves it.
#[cfg(any(test, feature = "testing"))]
impl State {
    /// Give the save actor `id`, with `record` and no components yet.
    pub fn add_actor(&mut self, id: &str, record: crate::model::ActorLive) -> Result<(), Reason> {
        if self.actors.records.contains_key(id) {
            return Err(Reason::Schema);
        }
        self.actors.records.put(id.to_string(), record.into());
        self.ledger.added(ComponentId::Actor(id.to_string()));
        Ok(())
    }

    /// Take actor `id` out of the save: each of their components through its writer, in the
    /// reverse of the order a copy puts them on, then their record, so a rollback puts them all
    /// back.
    pub fn remove_actor(&mut self, canon: &Canon, id: &str) -> Result<(), Reason> {
        if !self.actors.records.contains_key(id) {
            return Err(Reason::UnknownId);
        }
        for part in self.parts_of(id).into_iter().rev() {
            self.put_part(canon, id, part, false)?;
        }
        let record = self.actors.records.take(id).expect("an actor");
        self.ledger
            .removed(Removed::Actor(id.to_string(), Box::new(record)));
        Ok(())
    }

    /// Give the save actor `to`, a copy of actor `from`: their record, then each of their
    /// components through its writer.
    pub fn copy_actor(&mut self, canon: &Canon, from: &str, to: &str) -> Result<(), Reason> {
        let record = self
            .actors
            .records
            .get(from)
            .ok_or(Reason::UnknownId)?
            .clone();
        let parts = self.parts_of(from);
        self.add_actor(to, record)?;
        for part in parts {
            self.put_part(canon, to, part, true)?;
        }
        Ok(())
    }

    /// Every component actor `id` holds, in the order `ActorComponents::parts` walks them,
    /// which is the order a copy puts them on.
    fn parts_of(&self, id: &str) -> Vec<crate::model::ActorPart> {
        let parts = self.actors.parts().into_iter();
        parts
            .filter(|(on, _)| on == id)
            .map(|(_, part)| part)
            .collect()
    }

    /// Put `part` on actor `onto` through its own writer, or take it off them when `on` is
    /// false.
    fn put_part(
        &mut self,
        canon: &Canon,
        onto: &str,
        part: crate::model::ActorPart,
        on: bool,
    ) -> Result<(), Reason> {
        use crate::model::ActorPart;
        let entity = EntityId::Actor(onto.to_string());
        match part {
            ActorPart::Item(id, count) => self.set_items(&entity, &id, if on { count } else { 0 }),
            ActorPart::Meter(key, live) => self.put_meter(canon, &entity, &key, on.then_some(live)),
            ActorPart::Modifier(id, live) => {
                self.put_modifier(canon, onto, &id, on.then_some(live))
            }
            ActorPart::Flag(key) => self.set_flag(&entity, &key, on),
        }
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
        let bram = EntityId::Actor("bram".to_string());
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
                entity: EntityId::Actor(actor.to_string()),
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

    #[test]
    fn a_copy_holds_its_sources_items_and_removing_it_leaves_the_save_as_it_was() {
        let pack =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packs/oneshot-docks");
        let opened = World::open(&pack).expect("open");
        let canon = opened.canon();
        let (mira, briar) = (
            EntityId::Actor("mira".into()),
            EntityId::Actor("briar".into()),
        );
        let oil = |state: &State, entity: &EntityId| state.item_count(entity, "lamp_oil");
        let mut state = opened.state().clone();
        state.set_items(&mira, "lamp_oil", 2).unwrap();
        state.accept();
        let before = state.clone();
        state.copy_actor(canon, "mira", "briar").unwrap();
        assert_eq!(oil(&state, &briar), Some(2), "the copy's oil");
        assert_eq!(oil(&state, &mira), Some(2), "the source's oil");
        state.remove_actor(canon, "briar").unwrap();
        state.accept();
        assert_eq!(state, before);
    }
}
