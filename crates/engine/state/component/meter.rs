//! Meter bounds from the pack, and the live value on its entity.

use serde::{Deserialize, Serialize};

use crate::state::change::{self, Change};
use crate::state::entity;
use crate::Reason;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MeterDef {
    pub min: i64,
    pub base_cap: i64,
    pub max: i64,
    pub start: i64,
    /// Kept off the players' views.
    pub hidden: bool,
    /// Who carries it, from the file that declares it. `None` is every actor.
    pub carrier: Option<entity::Id>,
}

impl MeterDef {
    /// Whether `id` carries this meter.
    pub fn is_on(&self, id: entity::IdRef<'_>) -> bool {
        match &self.carrier {
            None => matches!(id, entity::IdRef::Actor(_)),
            Some(carrier) => carrier.as_ref() == id,
        }
    }

    /// Whether every actor, or one actor, carries this meter.
    pub fn is_actor_meter(&self) -> bool {
        matches!(self.carrier, None | Some(entity::Id::Actor(_)))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Meter {
    pub base: i64,
    pub offset: i64,
}

/// A change to a meter's offset. A journal line writes it as `"write":"offset","delta":<n>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "write", content = "delta", rename_all = "snake_case")]
pub enum MeterWrite {
    Offset(i64),
}

/// `base` is inside `[min, base_cap]`, and `base + offset` fits in `i64`.
/// A sum outside `[min, max]` still returns true. `value` clamps that.
pub fn is_storable(def: &MeterDef, base: i64, offset: i64) -> bool {
    (def.min..=def.base_cap).contains(&base) && base.checked_add(offset).is_some()
}

/// `base + offset`, plus what the entity's modifiers add to this meter, clamped to the bounds.
pub fn value(def: &MeterDef, live: &Meter, modifiers: i64) -> Result<i64, Reason> {
    let sum = live
        .base
        .checked_add(live.offset)
        .and_then(|sum| sum.checked_add(modifiers))
        .ok_or(Reason::OutOfRange)?;
    Ok(sum.clamp(def.min, def.max))
}

/// The current value of a meter no modifier reaches, or `UNKNOWN_ID` when the pack does not give
/// it or the save does not hold it.
pub fn read(def: Option<&MeterDef>, held: Option<&Meter>) -> Result<i64, Reason> {
    value(
        def.ok_or(Reason::UnknownId)?,
        held.ok_or(Reason::UnknownId)?,
        0,
    )
}

/// `held` with its offset moved by `delta` inside `def`'s bounds, and the change that reports
/// under `key`, its value read with `modifiers`: `UNKNOWN_ID` when nothing is held. A sum past
/// `i64` reads as no value and never refuses the write.
pub(in crate::state) fn shift(
    held: Option<&Meter>,
    def: &MeterDef,
    modifiers: Result<i64, Reason>,
    delta: i64,
    key: change::Key,
) -> Result<(Meter, Change), Reason> {
    let held = held.ok_or(Reason::UnknownId)?;
    let offset = held.offset.checked_add(delta).ok_or(Reason::OutOfRange)?;
    if !is_storable(def, held.base, offset) {
        return Err(Reason::OutOfRange);
    }
    let next = Meter { offset, ..*held };
    let reading =
        |held: &Meter| change::Value::from_reading(modifiers.and_then(|sum| value(def, held, sum)));
    let change = Change::new(key, reading(held), reading(&next));
    Ok((next, change))
}

/// `meter` in place of `held`, `None` taking it off, outside a round's rules, and the change that
/// reports under `key`, each value read with `def` and `modifiers`.
pub(in crate::state) fn next(
    held: Option<&Meter>,
    meter: Option<Meter>,
    def: Option<&MeterDef>,
    modifiers: Result<i64, Reason>,
    key: change::Key,
) -> (Option<Meter>, Change) {
    let reading = |held: Option<&Meter>| {
        let reading = (def.zip(held).ok_or(Reason::UnknownId))
            .and_then(|(def, held)| value(def, held, modifiers?));
        change::Value::from_reading(reading)
    };
    let change = Change::new(key, reading(held), reading(meter.as_ref()));
    (meter, change)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grit() -> MeterDef {
        MeterDef {
            min: 0,
            base_cap: 100,
            max: 110,
            start: 1,
            hidden: false,
            carrier: None,
        }
    }

    #[test]
    fn stored_accepts_a_sum_past_max_and_current_clamps() {
        let def = grit();
        assert!(is_storable(&def, 100, 20));
        let live = Meter {
            base: 100,
            offset: 20,
        };
        assert_eq!(value(&def, &live, 0).unwrap(), 110);
        assert_eq!(value(&def, &live, -15).unwrap(), 105);
    }

    #[test]
    fn stored_rejects_a_sum_that_does_not_fit() {
        assert!(!is_storable(&grit(), 3, i64::MAX));
    }
}
