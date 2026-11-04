//! Meter bounds from the pack, and the live value on its holder.

use serde::{Deserialize, Serialize};

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
    /// Whether actor `id` carries this meter.
    pub fn is_on_actor(&self, id: &str) -> bool {
        match &self.carrier {
            None => true,
            Some(Holder::Actor(carrier)) => carrier == id,
            Some(_) => false,
        }
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

/// `Ok(true)` means the stored pair changed and the caller should track
/// the meter's key. A zero delta leaves the pair alone and is `Ok(false)`.
/// The live record is assigned only after `is_storable` accepts the next pair.
pub fn write(def: &MeterDef, live: &mut MeterLive, write: MeterWrite) -> Result<bool, Reason> {
    let MeterWrite::Offset(delta) = write;
    let next_offset = live.offset.checked_add(delta).ok_or(Reason::OutOfRange)?;
    if !is_storable(def, live.base, next_offset) {
        return Err(Reason::OutOfRange);
    }
    let changed = next_offset != live.offset;
    live.offset = next_offset;
    Ok(changed)
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
