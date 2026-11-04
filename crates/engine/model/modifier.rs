//! Modifier defs from the pack, and the stacks an actor holds of each.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::Reason;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModifierDef {
    /// The actor meter it modifies.
    pub meter: String,
    /// World minutes it lasts after its latest application.
    pub minutes: u64,
    /// The most stacks one actor holds, from any source. At least 1.
    pub max_stacks: usize,
    /// Kept off the players' views.
    pub hidden: bool,
}

/// One modifier on one actor: the meter it was applied to, the world minute it wears off,
/// and each application's magnitude. The meter is kept so migrate can tell when the def
/// now names another one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModifierLive {
    pub meter: String,
    pub until: u64,
    pub stacks: Vec<i64>,
}

/// Apply `add` and set the end to `until`. Below `max_stacks` it is one more stack. At the
/// cap it replaces the first weakest stack when its absolute value is larger.
pub fn stack(def: &ModifierDef, live: &mut ModifierLive, add: i64, until: u64) {
    live.until = until;
    if live.stacks.len() < def.max_stacks {
        live.stacks.push(add);
        return;
    }
    let weakest = live
        .stacks
        .iter_mut()
        .min_by_key(|held| held.unsigned_abs());
    if let Some(weakest) = weakest.filter(|held| add.unsigned_abs() > held.unsigned_abs()) {
        *weakest = add;
    }
}

/// Whether `defs` still define modifier `id` on the meter `live` was applied to.
pub fn is_defined(defs: &BTreeMap<String, ModifierDef>, id: &str, live: &ModifierLive) -> bool {
    defs.get(id).is_some_and(|def| def.meter == live.meter)
}

/// What the modifiers in `held` add to meter `key`: the sum of their stacks.
pub fn sum(held: &BTreeMap<String, ModifierLive>, key: &str) -> Result<i64, Reason> {
    held.values()
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
            minutes: 10,
            max_stacks,
            hidden: false,
        }
    }

    fn held(stacks: &[i64]) -> ModifierLive {
        ModifierLive {
            meter: "grit".to_string(),
            until: 2,
            stacks: stacks.to_vec(),
        }
    }

    #[test]
    fn at_the_cap_a_tie_replaces_nothing_and_a_stronger_one_takes_the_first_weakest() {
        let mut live = held(&[-2, 1, -1]);
        stack(&burn(3), &mut live, -1, 12);
        assert_eq!((live.stacks.as_slice(), live.until), (&[-2, 1, -1][..], 12));
        stack(&burn(3), &mut live, 3, 14);
        assert_eq!((live.stacks.as_slice(), live.until), (&[-2, 3, -1][..], 14));
    }

    #[test]
    fn sum_counts_only_the_named_meter_and_refuses_an_overflow() {
        let mut map = BTreeMap::from([("burn".to_string(), held(&[-1, -2]))]);
        map.insert(
            "calm".to_string(),
            ModifierLive {
                meter: "rest".to_string(),
                until: 2,
                stacks: vec![5],
            },
        );
        assert_eq!(sum(&map, "grit"), Ok(-3));
        assert_eq!(sum(&map, "nerve"), Ok(0));
        map.insert("vast".to_string(), held(&[i64::MIN]));
        assert_eq!(sum(&map, "grit"), Err(Reason::OutOfRange));
    }
}
