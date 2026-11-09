//! The actors' components, one store per component, keyed by actor id.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::id::{ActorId, EntityRef};
use super::meter::{self, Meter};
use super::modifier::{self, Modifier};
use super::{change, flag, item, nested_map, sort_unique, Writer};
use crate::canon::Canon;
use crate::intent::{Target, Viewer};
use crate::Reason;

/// Every actor's components, by actor id. An actor exists when it has a kind; `kind`, `place`
/// and `acts` have an entry for every actor, the rest only for actors holding something.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ActorComponents {
    /// Whether each actor is a PC or an NPC. An actor exists when it has a kind.
    pub kind: BTreeMap<ActorId, ActorKind>,
    /// Where each actor stands: a place id, or `exit:<place>.<exit>` while they walk.
    pub place: BTreeMap<ActorId, String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub wielded: BTreeMap<ActorId, String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub known_facts: BTreeMap<ActorId, Vec<String>>,
    /// Each place an actor has stood at the end of a round, with the names of the exits they
    /// saw there then.
    #[serde(with = "nested_map", skip_serializing_if = "BTreeMap::is_empty")]
    pub visited: BTreeMap<(ActorId, String), Vec<String>>,
    pub acts: BTreeMap<ActorId, Act>,
    #[serde(with = "nested_map")]
    pub items: BTreeMap<(ActorId, String), u32>,
    #[serde(with = "nested_map")]
    pub meters: BTreeMap<(ActorId, String), Meter>,
    /// The modifiers each actor holds, by modifier id.
    #[serde(with = "nested_map")]
    pub modifiers: BTreeMap<(ActorId, String), Modifier>,
    #[serde(with = "nested_map::lists")]
    pub flags: BTreeSet<(ActorId, String)>,
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
        let def = canon.meter_def(EntityRef::Actor(actor), key);
        let held = self.meter(actor, key).ok_or(Reason::UnknownId);
        meter::current(
            def.ok_or(Reason::UnknownId)?,
            held?,
            self.modifier_sum(actor, key)?,
        )
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
}

impl Writer<'_, ActorComponents> {
    /// Stand `actor` at `to`: a place id, or an exit address while they walk.
    pub fn set_place(&mut self, actor: &ActorId, to: &str) -> Result<(), Reason> {
        let Writer { part, transaction } = self;
        part.kind.get(actor).ok_or(Reason::UnknownId)?;
        let place = part.place.get_mut(actor).ok_or(Reason::UnknownId)?;
        if place == to {
            return Ok(());
        }
        let before = change::Value::Text(std::mem::replace(place, to.to_string()));
        let after = change::Value::Text(to.to_string());
        transaction.report(change::Key::ActorPlace(actor.to_string()), before, after)
    }

    /// Wield `item`, which `actor` must hold, or with `None` let go of what they wield.
    pub fn set_wielded(&mut self, actor: &ActorId, item: Option<&str>) -> Result<(), Reason> {
        let Writer { part, transaction } = self;
        part.kind.get(actor).ok_or(Reason::UnknownId)?;
        if item.is_some_and(|item| nested_map::get(&part.items, actor, item).is_none()) {
            return Err(Reason::MissingItem);
        }
        let before = match item {
            Some(item) => part.wielded.insert(actor.clone(), item.to_string()),
            None => part.wielded.remove(actor),
        };
        if before.as_deref() == item {
            return Ok(());
        }
        let text = |item: Option<String>| item.map_or(change::Value::Empty, change::Value::Text);
        let after = text(item.map(str::to_string));
        transaction.report(
            change::Key::ActorWielded(actor.to_string()),
            text(before),
            after,
        )
    }

    /// `actor` knows `facts` from now on, kept in order with no repeats.
    pub fn learn(&mut self, actor: &ActorId, facts: &[String]) -> Result<(), Reason> {
        let Writer { part, transaction } = self;
        part.kind.get(actor).ok_or(Reason::UnknownId)?;
        let held = part.known_facts.get(actor);
        if facts
            .iter()
            .all(|fact| held.is_some_and(|held| held.contains(fact)))
        {
            return Ok(());
        }
        let before = change::Value::List(held.cloned().unwrap_or_default());
        let known = part.known_facts.entry(actor.clone()).or_default();
        known.extend_from_slice(facts);
        sort_unique(known);
        let after = change::Value::List(known.clone());
        transaction.report(
            change::Key::ActorKnownFact(actor.to_string()),
            before,
            after,
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
        let Writer { part, transaction } = self;
        part.kind.get(actor).ok_or(Reason::UnknownId)?;
        let act = part.acts.get_mut(actor).ok_or(Reason::UnknownId)?;
        if act.busy_until == Some(until) && act.pending == pending {
            return Ok(());
        }
        let before = act.busy_value();
        (act.busy_until, act.pending) = (Some(until), pending);
        let key = change::Key::ActorBusy(actor.to_string());
        transaction.report(key, before, act.busy_value())
    }

    /// End `actor`'s action under way, if any, and free them. The action, so `cancel` and
    /// `finish` see it.
    pub fn take_pending(&mut self, actor: &ActorId) -> Result<Option<PendingAction>, Reason> {
        let Writer { part, transaction } = self;
        part.kind.get(actor).ok_or(Reason::UnknownId)?;
        let act = part.acts.get_mut(actor).ok_or(Reason::UnknownId)?;
        let before = act.busy_value();
        if act.busy_until.take().is_some() | act.pending.is_some() {
            let key = change::Key::ActorBusy(actor.to_string());
            transaction.report(key, before, change::Value::Empty)?;
        }
        Ok(act.pending.take())
    }

    /// Count one act for `actor` in round `round_index`, in the act window the table is in: the
    /// idle one when no player is seated. Only the keys whose values move are reported.
    pub fn count_act(
        &mut self,
        actor: &ActorId,
        round_index: u64,
        players: u32,
    ) -> Result<(), Reason> {
        let Writer { part, transaction } = self;
        part.kind.get(actor).ok_or(Reason::UnknownId)?;
        let act = part.acts.get_mut(actor).ok_or(Reason::UnknownId)?;
        let counter = match players {
            0 => ActCounter::IdleActsThisWindow,
            _ => ActCounter::LiveActsThisWindow,
        };
        let last = std::mem::replace(&mut act.last_act_round, round_index);
        let count = counter.count_mut(act);
        let counted = std::mem::replace(count, count.saturating_add(1));
        if last != round_index {
            let key = change::Key::LastActRound(actor.to_string());
            transaction.report(key, number(last), number(round_index))?;
        }
        if *count != counted {
            let key = counter.key(actor.to_string());
            transaction.report(key, number(counted.into()), number((*count).into()))?;
        }
        Ok(())
    }

    /// Set `counter` back to 0 on every actor.
    pub(crate) fn reset_acts(&mut self, counter: ActCounter) -> Result<(), Reason> {
        let Writer { part, transaction } = self;
        for actor in part.kind.keys() {
            let act = part.acts.get_mut(actor).ok_or(Reason::UnknownId)?;
            let count = std::mem::take(counter.count_mut(act));
            if count != 0 {
                let key = counter.key(actor.to_string());
                transaction.report(key, number(count.into()), number(0))?;
            }
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
        self.part.kind.get(actor).ok_or(Reason::UnknownId)?;
        self.part
            .visited
            .insert((actor.clone(), place.to_string()), exits);
        Ok(())
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
        let Writer { part, transaction } = self;
        part.kind.get(actor).ok_or(Reason::UnknownId)?;
        let entry = (actor.clone(), id.to_string());
        let key = change::Key::ActorItem(actor.to_string(), id.to_string());
        item::shift(&mut part.items, transaction, entry, delta, max, key)?;
        let wielded = part.wielded.get(actor);
        if wielded.is_some_and(|held| nested_map::get(&part.items, actor, held).is_none()) {
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
        let Writer { part, transaction } = self;
        part.kind.get(actor).ok_or(Reason::UnknownId)?;
        let def = canon
            .meter_def(EntityRef::Actor(actor), key)
            .ok_or(Reason::UnknownId)?;
        let modifiers = part.modifier_sum(actor, key);
        let held = nested_map::get_mut(&mut part.meters, actor, key).ok_or(Reason::UnknownId)?;
        let watched = change::Key::ActorMeter(actor.to_string(), key.to_string());
        meter::shift(held, transaction, def, modifiers, delta, watched)
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
        let Writer { part, transaction } = self;
        if meter.is_some() {
            part.kind.get(actor).ok_or(Reason::UnknownId)?;
        }
        let def = canon.meter_def(EntityRef::Actor(actor), key);
        let modifiers = part.modifier_sum(actor, key);
        let entry = (actor.clone(), key.to_string());
        let watched = change::Key::ActorMeter(actor.to_string(), key.to_string());
        meter::set(
            &mut part.meters,
            transaction,
            entry,
            meter,
            def,
            modifiers,
            watched,
        )
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
        let Writer { part, transaction } = self;
        let held = nested_map::get(&part.modifiers, actor, id);
        let meter = match (&next, held) {
            (Some(_), _) => {
                part.kind.get(actor).ok_or(Reason::UnknownId)?;
                canon
                    .modifiers
                    .get(id)
                    .ok_or(Reason::UnknownId)?
                    .meter
                    .clone()
            }
            (None, Some(held)) => held.meter.clone(),
            (None, None) => return Ok(()),
        };
        let reading = |part: &ActorComponents| {
            change::Value::from_reading(part.meter_value(canon, actor, &meter))
        };
        let own = |held: Option<&Modifier>| held.map_or(change::Value::Empty, Modifier::to_value);
        let (before, own_before, own_after) = (reading(part), own(held), own(next.as_ref()));
        let stacks_move = held.map(|held| &held.stacks) != next.as_ref().map(|next| &next.stacks);
        let changed = held != next.as_ref();
        let watched = change::Key::ActorMeter(actor.to_string(), meter.clone());
        if stacks_move {
            transaction.count_key(&watched)?;
        }
        let entry = (actor.clone(), id.to_string());
        if let Some(next) = next {
            part.modifiers.insert(entry, next);
            part.meter_value(canon, actor, &meter)?;
        } else {
            part.modifiers.remove(&entry);
        }
        if stacks_move {
            transaction.report(watched, before, reading(part))?;
        }
        if changed {
            let key = change::Key::ActorModifier(actor.to_string(), id.to_string());
            transaction.report(key, own_before, own_after)?;
        }
        Ok(())
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
        let Writer { part, transaction } = self;
        part.kind.get(actor).ok_or(Reason::UnknownId)?;
        let entry = (actor.clone(), key.to_string());
        let watched = change::Key::ActorFlag(actor.to_string(), key.to_string());
        flag::set(&mut part.flags, transaction, entry, on, watched)
    }
}

/// A count as a change value.
fn number(count: u64) -> change::Value {
    change::Value::Number(i64::try_from(count).unwrap_or(i64::MAX))
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
        self.busy_until.map_or(change::Value::Empty, number)
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
