//! The actors' components, one store per component, keyed by actor id.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::change::{self, Change};
use super::id::{ActorId, EntityRef};
use super::meter::{self, Meter};
use super::modifier::{self, Modifier};
use super::transaction::Transaction;
use super::undo::LogCut;
use super::{flag, item, nested_map, sort_unique, DenseStore, Store, Writer};
use crate::canon::Canon;
use crate::intent::{Target, Viewer};
use crate::Reason;

/// Every actor's components, by actor id. An actor exists when it has a kind; `kind`, `place`
/// and `acts` have an entry for every actor, the rest only for actors holding something.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ActorComponents {
    /// Whether each actor is a PC or an NPC. An actor exists when it has a kind.
    pub kind: Store<BTreeMap<ActorId, ActorKind>>,
    /// Where each actor stands: a place id, or `exit:<place>.<exit>` while they walk.
    pub place: DenseStore<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub wielded: Store<BTreeMap<ActorId, String>>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub known_facts: Store<BTreeMap<ActorId, Vec<String>>>,
    /// Each place an actor has stood at the end of a round, with the names of the exits they
    /// saw there then.
    #[serde(with = "nested_map", skip_serializing_if = "BTreeMap::is_empty")]
    pub visited: Store<BTreeMap<(ActorId, String), Vec<String>>>,
    pub acts: DenseStore<Act>,
    #[serde(with = "nested_map")]
    pub items: Store<BTreeMap<(ActorId, String), u32>>,
    #[serde(with = "nested_map")]
    pub meters: Store<BTreeMap<(ActorId, String), Meter>>,
    /// The modifiers each actor holds, by modifier id.
    #[serde(with = "nested_map")]
    pub modifiers: Store<BTreeMap<(ActorId, String), Modifier>>,
    #[serde(with = "nested_map::lists")]
    pub flags: Store<BTreeSet<(ActorId, String)>>,
}

impl ActorComponents {
    /// Meter `key` as `actor` holds it, before modifiers.
    pub fn meter(&self, actor: &ActorId, key: &str) -> Option<&Meter> {
        nested_map::get(&self.meters, actor, key)
    }

    /// What `actor`'s modifiers add to meter `key`.
    pub fn modifier_sum(&self, actor: &ActorId, key: &str) -> Result<i64, Reason> {
        let held = nested_map::entries_of(&self.modifiers, actor).map(|(_, held)| held);
        modifier::sum(held, key)
    }

    /// The current value of meter `key` on `actor`, or `UNKNOWN_ID` when they do not carry it.
    pub fn meter_value(&self, canon: &Canon, actor: &ActorId, key: &str) -> Result<i64, Reason> {
        self.meter_value_with(canon, actor, key, self.modifier_sum(actor, key))
    }

    /// The value of meter `key` on `actor` with `modifiers` added, for a write that has not
    /// landed: the same refusals as `meter_value`, in the same order.
    fn meter_value_with(
        &self,
        canon: &Canon,
        actor: &ActorId,
        key: &str,
        modifiers: Result<i64, Reason>,
    ) -> Result<i64, Reason> {
        let def = canon.meter_def(EntityRef::Actor(actor), key);
        let held = self.meter(actor, key).ok_or(Reason::UnknownId);
        meter::current(def.ok_or(Reason::UnknownId)?, held?, modifiers?)
    }

    /// How many of `item` `actor` holds, or `None` when there is no such actor.
    pub fn item_count(&self, actor: &ActorId, item: &str) -> Option<u32> {
        self.kind.get(actor)?;
        Some(
            nested_map::get(&self.items, actor, item)
                .copied()
                .unwrap_or(0),
        )
    }

    pub fn has_flag(&self, actor: &ActorId, key: &str) -> bool {
        nested_map::contains(&self.flags, actor, key)
    }

    /// Truncate every store's log as `cut` says.
    pub(super) fn truncate_logs(&mut self, cut: LogCut) {
        // Every store, with no `..`: a store added later must be truncated here to compile.
        let ActorComponents {
            kind,
            place,
            wielded,
            known_facts,
            visited,
            acts,
            items,
            meters,
            modifiers,
            flags,
        } = self;
        kind.truncate_log(cut);
        place.truncate_log(cut);
        wielded.truncate_log(cut);
        known_facts.truncate_log(cut);
        visited.truncate_log(cut);
        acts.truncate_log(cut);
        items.truncate_log(cut);
        meters.truncate_log(cut);
        modifiers.truncate_log(cut);
        flags.truncate_log(cut);
    }
}

impl Writer<'_, ActorComponents> {
    /// The components and the transaction, once `actor` is known to exist.
    fn check_actor(
        &mut self,
        actor: &ActorId,
    ) -> Result<(&mut ActorComponents, &mut Transaction), Reason> {
        self.components.kind.get(actor).ok_or(Reason::UnknownId)?;
        Ok((&mut *self.components, &mut *self.transaction))
    }

    /// The one write an actor writer makes: `actor` must exist; `set` puts the next value in its
    /// store with the change it reports, `key`'s from what `read` gives for the value held to
    /// `after`, the next value read the same way. A write that moves nothing reports nothing.
    fn write(
        &mut self,
        actor: &ActorId,
        key: change::Key,
        read: impl FnOnce(&ActorComponents) -> change::Value,
        after: change::Value,
        set: impl FnOnce(&mut ActorComponents, &mut Transaction, [Change; 1]) -> Result<(), Reason>,
    ) -> Result<(), Reason> {
        let (components, transaction) = self.check_actor(actor)?;
        let before = read(components);
        set(components, transaction, [Change::new(key, before, after)])
    }

    /// The one write to `actor`'s action state: `edit` made to what they hold, reporting `key`'s
    /// change as `read` reads the action state before and after.
    fn write_act(
        &mut self,
        actor: &ActorId,
        key: change::Key,
        read: impl Fn(&Act) -> change::Value,
        edit: impl FnOnce(&mut Act),
    ) -> Result<(), Reason> {
        let mut next = self.acts.get(actor).cloned().unwrap_or_default();
        edit(&mut next);
        let after = read(&next);
        let before = |components: &ActorComponents| {
            components
                .acts
                .get(actor)
                .map_or(change::Value::Empty, &read)
        };
        self.write(
            actor,
            key,
            before,
            after,
            |components, transaction, reports| {
                components.acts.set(transaction, actor, next, reports)
            },
        )
    }

    /// Stand `actor` at `to`: a place id, or an exit address while they walk.
    pub fn set_place(&mut self, actor: &ActorId, to: &str) -> Result<(), Reason> {
        let key = change::Key::ActorPlace(actor.to_string());
        let read = |components: &ActorComponents| text_value(components.place.get(actor));
        let after = text_value(Some(to));
        self.write(
            actor,
            key,
            read,
            after,
            |components, transaction, reports| {
                components
                    .place
                    .set(transaction, actor, to.to_string(), reports)
            },
        )
    }

    /// Wield `item`, which `actor` must hold, or with `None` let go of what they wield.
    pub fn set_wielded(&mut self, actor: &ActorId, item: Option<&str>) -> Result<(), Reason> {
        let key = change::Key::ActorWielded(actor.to_string());
        let read = |components: &ActorComponents| text_value(components.wielded.get(actor));
        self.write(
            actor,
            key,
            read,
            text_value(item),
            |components, transaction, reports| {
                if item
                    .is_some_and(|item| nested_map::get(&components.items, actor, item).is_none())
                {
                    return Err(Reason::MissingItem);
                }
                components.wielded.set(
                    transaction,
                    actor.clone(),
                    item.map(str::to_string),
                    reports,
                )
            },
        )
    }

    /// `actor` knows `facts` from now on, kept in order with no repeats.
    pub fn learn(&mut self, actor: &ActorId, facts: &[String]) -> Result<(), Reason> {
        let key = change::Key::ActorKnownFact(actor.to_string());
        let read = |components: &ActorComponents| {
            change::Value::List(
                components
                    .known_facts
                    .get(actor)
                    .cloned()
                    .unwrap_or_default(),
            )
        };
        let mut next = self.known_facts.get(actor).cloned().unwrap_or_default();
        let unknown = facts.iter().any(|fact| !next.contains(fact));
        next.extend_from_slice(facts);
        sort_unique(&mut next);
        let after = change::Value::List(next.clone());
        self.write(
            actor,
            key,
            read,
            after,
            |components, transaction, reports| {
                if !unknown {
                    return Ok(());
                }
                components
                    .known_facts
                    .set(transaction, actor.clone(), Some(next), reports)
            },
        )
    }

    /// Keep `actor` busy until `until`, on `pending` while an action is under way and on `None`
    /// through a cooldown.
    pub fn set_busy(
        &mut self,
        actor: &ActorId,
        until: u64,
        pending: Option<PendingAction>,
    ) -> Result<(), Reason> {
        let key = change::Key::ActorBusy(actor.to_string());
        self.write_act(actor, key, Act::busy_value, |act| {
            (act.busy_until, act.pending) = (Some(until), pending);
        })
    }

    /// End `actor`'s action under way, if any, and free them. The action, so `cancel` and
    /// `finish` see it.
    pub fn take_pending(&mut self, actor: &ActorId) -> Result<Option<PendingAction>, Reason> {
        let (key, mut taken) = (change::Key::ActorBusy(actor.to_string()), None);
        self.write_act(actor, key, Act::busy_value, |act| {
            taken = act.pending.take();
            act.busy_until = None;
        })?;
        Ok(taken)
    }

    /// Count one act for `actor` in round `round_index`, in the act window the table is in: the
    /// idle one when no player is seated. Only the keys whose values move are reported.
    pub fn count_act(
        &mut self,
        actor: &ActorId,
        round_index: u64,
        players: u32,
    ) -> Result<(), Reason> {
        let key = change::Key::LastActRound(actor.to_string());
        let read = |act: &Act| count_value(act.last_act_round);
        self.write_act(actor, key, read, |act| act.last_act_round = round_index)?;
        let counter = match players {
            0 => ActCounter::IdleActsThisWindow,
            _ => ActCounter::LiveActsThisWindow,
        };
        let read = |act: &Act| count_value(counter.count(act).into());
        self.write_act(actor, counter.key(actor.to_string()), read, |act| {
            let count = counter.count_mut(act);
            *count = count.saturating_add(1);
        })
    }

    /// Set `counter` back to 0 on every actor.
    pub(crate) fn reset_acts(&mut self, counter: ActCounter) -> Result<(), Reason> {
        let actors: Vec<ActorId> = self.kind.keys().cloned().collect();
        for actor in &actors {
            let read = |act: &Act| count_value(counter.count(act).into());
            let key = counter.key(actor.to_string());
            self.write_act(actor, key, read, |act| *counter.count_mut(act) = 0)?;
        }
        Ok(())
    }

    /// `actor` saw `exits` at `place`, for `map`. No key answers for it, so nothing is reported.
    pub fn note_exits(
        &mut self,
        actor: &ActorId,
        place: &str,
        exits: Vec<String>,
    ) -> Result<(), Reason> {
        let (components, transaction) = self.check_actor(actor)?;
        let entry = (actor.clone(), place.to_string());
        components.visited.set(transaction, entry, Some(exits), [])
    }

    /// Move `actor`'s count of `id` as `item::shift_count` says. An actor who no longer holds
    /// the item they wield lets go of it.
    pub fn shift_item(
        &mut self,
        actor: &ActorId,
        id: &str,
        delta: i64,
        max: Option<i64>,
    ) -> Result<(), Reason> {
        let (components, transaction) = self.check_actor(actor)?;
        let entry = (actor.clone(), id.to_string());
        let key = change::Key::ActorItem(actor.to_string(), id.to_string());
        let (next, change) = item::shift(components.items.get(&entry).copied(), delta, max, key)?;
        components.items.set(transaction, entry, next, [change])?;
        let wielded = self.wielded.get(actor);
        if wielded.is_some_and(|held| nested_map::get(&self.items, actor, held).is_none()) {
            self.set_wielded(actor, None)?;
        }
        Ok(())
    }

    /// Move `actor`'s meter `key` by `delta` inside its bounds.
    pub fn shift_meter(
        &mut self,
        canon: &Canon,
        actor: &ActorId,
        key: &str,
        delta: i64,
    ) -> Result<(), Reason> {
        let (components, transaction) = self.check_actor(actor)?;
        let def = canon
            .meter_def(EntityRef::Actor(actor), key)
            .ok_or(Reason::UnknownId)?;
        let modifiers = components.modifier_sum(actor, key);
        let entry = (actor.clone(), key.to_string());
        let watched = change::Key::ActorMeter(actor.to_string(), key.to_string());
        let (next, change) = meter::shift(
            components.meters.get(&entry),
            def,
            modifiers,
            delta,
            watched,
        )?;
        components
            .meters
            .set(transaction, entry, Some(next), [change])
    }

    /// Give `actor` meter `key` as `meter` says, `None` taking it off, outside a round's rules:
    /// migrate and fixtures. Taking one off checks no owner.
    pub fn set_meter(
        &mut self,
        canon: &Canon,
        actor: &ActorId,
        key: &str,
        meter: Option<Meter>,
    ) -> Result<(), Reason> {
        // Taking a meter off checks no owner: the entry is its own evidence.
        let (components, transaction) = match meter {
            Some(_) => self.check_actor(actor)?,
            None => (&mut *self.components, &mut *self.transaction),
        };
        let def = canon.meter_def(EntityRef::Actor(actor), key);
        let modifiers = components.modifier_sum(actor, key);
        let entry = (actor.clone(), key.to_string());
        let watched = change::Key::ActorMeter(actor.to_string(), key.to_string());
        let held = components.meters.get(&entry);
        let (next, change) = meter::next(held, meter, def, modifiers, watched);
        components.meters.set(transaction, entry, next, [change])
    }

    /// Put `next` on as modifier `id`, `None` taking it off. The meter's key is counted when the
    /// stacks move, the meter must still read with `next` on, then the modifier's own key: the
    /// order the budget refuses in. Taking one off checks no owner.
    pub fn set_modifier(
        &mut self,
        canon: &Canon,
        actor: &ActorId,
        id: &str,
        next: Option<Modifier>,
    ) -> Result<(), Reason> {
        let meter = match (&next, nested_map::get(&self.modifiers, actor, id)) {
            (Some(_), _) => canon
                .modifiers
                .get(id)
                .ok_or(Reason::UnknownId)?
                .meter
                .clone(),
            (None, Some(held)) => held.meter.clone(),
            (None, None) => return Ok(()),
        };
        // Taking a modifier off checks no owner: the entry is its own evidence.
        let (components, transaction) = match next {
            Some(_) => self.check_actor(actor)?,
            None => (&mut *self.components, &mut *self.transaction),
        };
        let held = nested_map::get(&components.modifiers, actor, id);
        let own = |held: Option<&Modifier>| held.map_or(change::Value::Empty, Modifier::to_value);
        let own_key = change::Key::ActorModifier(actor.to_string(), id.to_string());
        let own = Change::new(own_key, own(held), own(next.as_ref()));
        let stacks_move = held.map(|held| &held.stacks) != next.as_ref().map(|next| &next.stacks);
        let watched = change::Key::ActorMeter(actor.to_string(), meter.clone());
        if stacks_move {
            transaction.count_key(&watched)?;
        }
        // The meter as it reads with `next` in place of what is held, which must read when it
        // goes on.
        let others =
            nested_map::entries_of(&components.modifiers, actor).filter(|(held, _)| *held != id);
        let sum = modifier::sum(others.map(|(_, held)| held).chain(&next), &meter);
        let after = components.meter_value_with(canon, actor, &meter, sum);
        if next.is_some() {
            after?;
        }
        let before = change::Value::from_reading(components.meter_value(canon, actor, &meter));
        let meter_change = Change::new(watched, before, change::Value::from_reading(after));
        let reports = stacks_move.then_some(meter_change).into_iter().chain([own]);
        let entry = (actor.clone(), id.to_string());
        components.modifiers.set(transaction, entry, next, reports)
    }

    /// Take off every modifier whose tick has come by `world_time`, all its stacks at once.
    pub fn expire_modifiers(&mut self, canon: &Canon, world_time: u64) -> Result<(), Reason> {
        let due: Vec<(ActorId, String)> = (self.modifiers.iter())
            .filter(|(_, held)| held.until <= world_time)
            .map(|(due, _)| due.clone())
            .collect();
        for (actor, id) in due {
            self.set_modifier(canon, &actor, &id, None)?;
        }
        Ok(())
    }

    pub fn set_flag(&mut self, actor: &ActorId, key: &str, on: bool) -> Result<(), Reason> {
        let (components, transaction) = self.check_actor(actor)?;
        let entry = (actor.clone(), key.to_string());
        let watched = change::Key::ActorFlag(actor.to_string(), key.to_string());
        let (next, change) = flag::next(on, watched);
        components.flags.set(transaction, entry, next, [change])
    }
}

/// A count as a change value.
fn count_value(count: u64) -> change::Value {
    change::Value::Number(i64::try_from(count).unwrap_or(i64::MAX))
}

/// A component's text as a change value, `Empty` when there is none.
fn text_value(held: Option<impl AsRef<str>>) -> change::Value {
    held.map_or(change::Value::Empty, |held| {
        change::Value::Text(held.as_ref().to_string())
    })
}

/// Whether an actor is a PC or an NPC.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorKind {
    Npc,
    Pc,
}

impl ActorKind {
    /// The seat actor `id` of this kind acts from.
    pub fn viewer(self, id: &ActorId) -> Viewer {
        match self {
            ActorKind::Pc => Viewer::Player(id.clone()),
            ActorKind::Npc => Viewer::Npc(id.clone()),
        }
    }
}

/// How many times an actor has acted, and the action they are still on.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Act {
    pub last_act_round: u64,
    pub idle_acts_this_window: u32,
    pub live_acts_this_window: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub busy_until: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<PendingAction>,
}

impl Act {
    /// The tick the actor is busy until, as a change value.
    fn busy_value(&self) -> change::Value {
        self.busy_until.map_or(change::Value::Empty, count_value)
    }

    /// Busy means an action under way, or a cooldown whose `busy_until` is still after this
    /// tick. An action stays under way until the round that reaches its finish ends.
    pub fn is_busy(&self, world_time: u64) -> bool {
        self.pending.is_some() || self.busy_until.is_some_and(|until| until > world_time)
    }
}

/// One of the `Act` counts the end of its act window sets back to 0.
#[derive(Clone, Copy)]
pub(crate) enum ActCounter {
    IdleActsThisWindow,
    LiveActsThisWindow,
}

impl ActCounter {
    /// The key this count is tracked under for actor `id`.
    pub(crate) fn key(self, id: String) -> change::Key {
        match self {
            ActCounter::IdleActsThisWindow => change::Key::IdleActsThisWindow(id),
            ActCounter::LiveActsThisWindow => change::Key::LiveActsThisWindow(id),
        }
    }

    /// This count on `act`.
    pub(crate) fn count(self, act: &Act) -> u32 {
        match self {
            ActCounter::IdleActsThisWindow => act.idle_acts_this_window,
            ActCounter::LiveActsThisWindow => act.live_acts_this_window,
        }
    }

    /// This count on `act`, to write.
    pub(crate) fn count_mut(self, act: &mut Act) -> &mut u32 {
        match self {
            ActCounter::IdleActsThisWindow => &mut act.idle_acts_this_window,
            ActCounter::LiveActsThisWindow => &mut act.live_acts_this_window,
        }
    }
}

/// A timed action that finishes when `busy_until` is reached: its verb, who queued it, the
/// target it was queued against, and the rule it started with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingAction {
    pub verb: String,
    pub source: crate::intent::Source,
    #[serde(default, skip_serializing_if = "Target::is_empty")]
    pub target: Target,
    pub rule: String,
}
