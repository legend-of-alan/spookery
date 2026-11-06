//! Meter bounds from the pack, and the live value on its holder.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::model::tracked::Edit;
use crate::rules::effect::Holder;
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
    pub carrier: Option<Holder>,
}

impl MeterDef {
    /// Whether `holder` carries this meter.
    pub fn is_on(&self, holder: &Holder) -> bool {
        match &self.carrier {
            None => matches!(holder, Holder::Actor(_)),
            Some(carrier) => carrier == holder,
        }
    }

    /// Whether every actor, or one actor, carries this meter.
    pub fn is_actor_meter(&self) -> bool {
        matches!(self.carrier, None | Some(Holder::Actor(_)))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeterLive {
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
/// A sum outside `[min, max]` still returns true. `current` clamps that.
pub fn is_storable(def: &MeterDef, base: i64, offset: i64) -> bool {
    (def.min..=def.base_cap).contains(&base) && base.checked_add(offset).is_some()
}

/// `base + offset`, plus what the holder's modifiers add to this meter, clamped to the bounds.
pub fn current(def: &MeterDef, live: &MeterLive, modifiers: i64) -> Result<i64, Reason> {
    let sum = live
        .base
        .checked_add(live.offset)
        .and_then(|sum| sum.checked_add(modifiers))
        .ok_or(Reason::OutOfRange)?;
    Ok(sum.clamp(def.min, def.max))
}

/// The edit that moves meter `key`'s offset by the write's delta, once `is_storable` accepts
/// the next pair.
pub(crate) fn write(
    def: &MeterDef,
    key: &str,
    live: &MeterLive,
    write: MeterWrite,
) -> Result<Offset, Reason> {
    let MeterWrite::Offset(by) = write;
    let next_offset = live.offset.checked_add(by).ok_or(Reason::OutOfRange)?;
    if !is_storable(def, live.base, next_offset) {
        return Err(Reason::OutOfRange);
    }
    Ok(Offset {
        key: key.to_string(),
        by,
    })
}

/// Meter `key`'s offset moves by `by`, a move `write` has checked. Its undo moves it back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offset {
    pub(crate) key: String,
    pub(crate) by: i64,
}

impl Edit<BTreeMap<String, MeterLive>> for Offset {
    fn apply(self, meters: &mut BTreeMap<String, MeterLive>) -> Option<Self> {
        if self.by == 0 {
            return None;
        }
        let live = meters.get_mut(&self.key)?;
        live.offset = live.offset.checked_add(self.by)?;
        Some(Offset {
            key: self.key,
            by: -self.by,
        })
    }
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
        let live = MeterLive {
            base: 100,
            offset: 20,
        };
        assert_eq!(current(&def, &live, 0).unwrap(), 110);
        assert_eq!(current(&def, &live, -15).unwrap(), 105);
    }

    #[test]
    fn stored_rejects_a_sum_that_does_not_fit() {
        assert!(!is_storable(&grit(), 3, i64::MAX));
    }
}
