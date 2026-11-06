//! Modifier defs from the pack, and the stacks an actor holds of each.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::model::key::StateValue;
use crate::model::tracked::{swap, Edit, Store};
use crate::rules::effect::Holder;
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
pub struct ModifierLive {
    pub meter: String,
    /// The tick it expires at.
    pub until: u64,
    /// The longest `ticks` applied so far, the most ticks a refresh leaves.
    pub max_ticks: u64,
    pub stacks: Vec<i64>,
}

/// Every modifier each actor holds, by the modifier's id. An edit is the whole next value.
pub type Modifiers = Store<ModifierLive, ModifierLive>;

/// A modifier's next value, whose undo is the value it replaced.
impl Edit<ModifierLive> for ModifierLive {
    fn apply(self, live: &mut ModifierLive) -> Option<Self> {
        swap(live, self)
    }
}

impl ModifierLive {
    /// What its key reads: its stacks and the tick it expires at.
    pub(crate) fn to_value(&self) -> StateValue {
        StateValue::Text(format!(
            "{:?} until {} max_ticks {}",
            self.stacks, self.until, self.max_ticks
        ))
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
pub fn stack(def: &ModifierDef, live: &mut ModifierLive, add: i64, ticks: u64, now: u64) {
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
pub fn is_defined(defs: &BTreeMap<String, ModifierDef>, id: &str, live: &ModifierLive) -> bool {
    defs.get(id).is_some_and(|def| def.meter == live.meter)
}

/// What the modifiers `holder` holds add to its meter `key`: the sum of their stacks.
pub fn sum(modifiers: &Modifiers, holder: &Holder, key: &str) -> Result<i64, Reason> {
    modifiers
        .on(holder)
        .map(|(_, live)| live)
        .filter(|live| live.meter == key)
        .flat_map(|live| &live.stacks)
        .try_fold(0i64, |total, add| total.checked_add(*add))
        .ok_or(Reason::OutOfRange)
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

    fn held(stacks: &[i64]) -> ModifierLive {
        ModifierLive {
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
        let rowan = Holder::Actor("rowan".to_string());
        let mut map = Modifiers::default();
        map.insert(rowan.clone(), "burn".to_string(), held(&[-1, -2]));
        map.insert(
            rowan.clone(),
            "calm".to_string(),
            ModifierLive {
                meter: "rest".to_string(),
                until: 2,
                max_ticks: 10,
                stacks: vec![5],
            },
        );
        assert_eq!(sum(&map, &rowan, "grit"), Ok(-3));
        assert_eq!(sum(&map, &rowan, "nerve"), Ok(0));
        map.insert(rowan.clone(), "vast".to_string(), held(&[i64::MIN]));
        assert_eq!(sum(&map, &rowan, "grit"), Err(Reason::OutOfRange));
    }
}
