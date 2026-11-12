//! Modifier defs from the pack, the stacks an actor holds of each, and an actor's meter as it
//! reads with them.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::state::change::{self, Change};
use crate::state::component::meter::{self, Meter};
use crate::state::component::{Component, Mut};
use crate::state::entity;
use crate::state::id::ActorId;
use crate::state::{Reader, Writer};
use crate::Reason;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModifierDef {
    /// The actor meter it modifies.
    pub meter: String,
    /// The most stacks one actor holds, from any source. At least 1.
    pub max_stacks: usize,
    /// Kept off the players' views.
    pub hidden: bool,
}

/// One modifier on one actor: the meter it was applied to, the tick it expires at, the
/// longest `ticks` applied so far, and each application's magnitude. The meter is kept so migrate
/// can tell when the def now names another one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Modifier {
    pub meter: String,
    /// The tick it expires at.
    pub until: u64,
    /// The longest `ticks` applied so far, the most ticks a refresh leaves.
    pub max_ticks: u64,
    pub stacks: Vec<i64>,
}

impl Component for Modifier {
    type Value = Modifier;
}

impl Modifier {
    /// The modifier as a change value: its stacks, the tick it expires at and its longest ticks.
    pub(crate) fn to_value(&self) -> crate::state::change::Value {
        let (stacks, until, max_ticks) = (&self.stacks, self.until, self.max_ticks);
        crate::state::change::Value::Text(format!("{stacks:?} until {until} max_ticks {max_ticks}"))
    }

    /// Ticks from `now` until it expires.
    pub fn ticks_left(&self, now: u64) -> u64 {
        self.until.saturating_sub(now)
    }
}

/// Apply `add`, lasting `ticks`, at tick `now`. The ticks left plus `ticks` are the new
/// ticks left, capped at `max_ticks`, the longest `ticks` so far, this one included.
/// Below `max_stacks` it is one more stack. At the cap it replaces the first weakest
/// stack when its absolute value is larger. Stacks past a `max_stacks` the pack lowered
/// go first, weakest first.
pub fn stack(def: &ModifierDef, live: &mut Modifier, add: i64, ticks: u64, now: u64) {
    live.max_ticks = live.max_ticks.max(ticks);
    let left = live.ticks_left(now).saturating_add(ticks);
    live.until = now.saturating_add(left.min(live.max_ticks));
    while live.stacks.len() > def.max_stacks {
        if let Some(index) = weakest(&live.stacks) {
            live.stacks.remove(index);
        }
    }
    if live.stacks.len() < def.max_stacks {
        live.stacks.push(add);
        return;
    }
    let stronger = weakest(&live.stacks)
        .filter(|&index| add.unsigned_abs() > live.stacks[index].unsigned_abs());
    if let Some(index) = stronger {
        live.stacks[index] = add;
    }
}

/// The index of the first stack with the smallest absolute value, in one pass.
fn weakest(stacks: &[i64]) -> Option<usize> {
    stacks
        .iter()
        .enumerate()
        .min_by_key(|(_, held)| held.unsigned_abs())
        .map(|(index, _)| index)
}

/// Whether `defs` still define modifier `id` on the meter `live` was applied to.
pub fn is_defined(defs: &BTreeMap<String, ModifierDef>, id: &str, live: &Modifier) -> bool {
    defs.get(id).is_some_and(|def| def.meter == live.meter)
}

/// What the modifiers in `held` add to meter `key`: the sum of their stacks.
pub fn sum<'a>(held: impl Iterator<Item = &'a Modifier>, key: &str) -> Result<i64, Reason> {
    held.filter(|live| live.meter == key)
        .flat_map(|live| &live.stacks)
        .try_fold(0i64, |total, add| total.checked_add(*add))
        .ok_or(Reason::OutOfRange)
}

/// Meter `name` on `actor` as rolls, costs, conditions and views read it: the stored meter plus
/// what the actor's modifiers add, inside the pack's bounds. `UNKNOWN_ID` when the actor does not
/// carry the meter.
pub fn value(
    actors: Reader<'_, entity::Actors>,
    actor: &ActorId,
    name: &str,
) -> Result<i64, Reason> {
    value_with(actors, actor, name, actors.modifier_sum(actor, name))
}

/// The same with `modifiers` in place of what the actor's modifiers add, for a write that has not
/// landed: the same refusals as `value`, in the same order.
pub(in crate::state) fn value_with(
    actors: Reader<'_, entity::Actors>,
    actor: &ActorId,
    name: &str,
    modifiers: Result<i64, Reason>,
) -> Result<i64, Reason> {
    let def = actors.canon.meter_def(entity::IdRef::Actor(actor), name);
    let held = actors.get::<Meter>(actor).get(name);
    let held = held.ok_or(Reason::UnknownId);
    meter::value(def.ok_or(Reason::UnknownId)?, held?, modifiers?)
}

impl Mut<'_, entity::Actors, Modifier> {
    /// Put `next` on as modifier `id`, `None` taking it off. The meter's key is counted when the
    /// stacks move, the meter must still read with `next` on, then the modifier's own key: the
    /// order the budget refuses in. When the stacks move it reports the meter's key with what the
    /// modifiers added before, and the change is read when the changes are taken. Taking one off
    /// checks no owner: the entry is its own evidence.
    pub fn set(&mut self, id: &str, next: Option<Modifier>) -> Result<(), Reason> {
        let (canon, actor) = (self.writer.canon, self.id);
        let actors = Reader::new(&*self.writer.components, canon);
        let held = actors.get::<Modifier>(actor).get(id);
        let meter = match (&next, held) {
            (Some(_), _) => canon
                .modifiers
                .get(id)
                .ok_or(Reason::UnknownId)?
                .meter
                .clone(),
            (None, Some(held)) => held.meter.clone(),
            (None, None) => return Ok(()),
        };
        if next.is_some() {
            self.check_exists()?;
        }
        let own = |held: Option<&Modifier>| held.map_or(change::Value::Empty, Modifier::to_value);
        let own_key = change::Key::ActorModifier(actor.to_string(), id.to_string());
        let own = Change::new(own_key, own(held), own(next.as_ref()));
        let stacks_move = held.map(|held| &held.stacks) != next.as_ref().map(|next| &next.stacks);
        let watched = change::Key::ActorMeter(actor.to_string(), meter.clone());
        if stacks_move {
            self.writer.transaction.count_key(&watched)?;
        }
        // The meter must read with `next` in place of what is held when it goes on.
        let others = (actors.get::<Modifier>(actor).iter()).filter(|(held, _)| *held != id);
        let with_next = sum(others.map(|(_, held)| held).chain(&next), &meter);
        if next.is_some() {
            value_with(actors, actor, &meter, with_next)?;
        }
        let before = actors.modifier_sum(actor, &meter);
        let watched = stacks_move.then_some(change::Report::Modifiers(watched, before));
        let reports = watched.into_iter().chain([own.into()]);
        let entry = (actor.clone(), id.to_string());
        (self.writer.components.modifiers).set(self.writer.transaction, entry, next, reports)
    }
}

impl Writer<'_, entity::Actors> {
    /// Take off every modifier whose tick has come by `world_time`, all its stacks at once.
    pub fn expire_modifiers(&mut self, world_time: u64) -> Result<(), Reason> {
        let due: Vec<(ActorId, String)> = (self.modifiers.iter())
            .filter(|(_, held)| held.until <= world_time)
            .map(|(due, _)| due.clone())
            .collect();
        for (actor, id) in due {
            self.get_mut::<Modifier>(&actor).set(&id, None)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn burn(max_stacks: usize) -> ModifierDef {
        ModifierDef {
            meter: "grit".to_string(),
            max_stacks,
            hidden: false,
        }
    }

    fn held(stacks: &[i64]) -> Modifier {
        Modifier {
            meter: "grit".to_string(),
            until: 2,
            max_ticks: 10,
            stacks: stacks.to_vec(),
        }
    }

    #[test]
    fn a_lowered_cap_drops_the_weakest_extra_stacks_on_the_next_application() {
        let mut live = held(&[-1, -3, -2, -1]);
        stack(&burn(2), &mut live, -1, 10, 2);
        assert_eq!(live.stacks, vec![-3, -2]);
    }

    #[test]
    fn sum_counts_only_the_named_meter_and_refuses_an_overflow() {
        let mut map = BTreeMap::from([("burn".to_string(), held(&[-1, -2]))]);
        map.insert(
            "calm".to_string(),
            Modifier {
                meter: "rest".to_string(),
                until: 2,
                max_ticks: 10,
                stacks: vec![5],
            },
        );
        assert_eq!(sum(map.values(), "grit"), Ok(-3));
        assert_eq!(sum(map.values(), "nerve"), Ok(0));
        map.insert("vast".to_string(), held(&[i64::MIN]));
        assert_eq!(sum(map.values(), "grit"), Err(Reason::OutOfRange));
    }
}
