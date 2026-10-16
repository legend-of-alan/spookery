use serde::{Deserialize, Serialize};

use crate::error::Reason;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MeterDef {
    pub min: i64,
    pub base_cap: i64,
    pub max: i64,
    pub start: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeterLive {
    pub base: i64,
    pub offset: i64,
}

impl MeterLive {
    pub(crate) fn at_start(def: &MeterDef) -> Self {
        Self {
            base: def.start,
            offset: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeterWrite {
    Base(i64),
    Offset(i64),
    Snap,
}

/// One cast work entry, or one literal meter template after the loader accepts it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkWrite {
    pub key: String,
    pub write: MeterWrite,
}

/// `base` is inside `[min, base_cap]`, and `base + offset` fits in `i64`.
/// A sum outside `[min, max]` still returns true. `current` clamps that.
pub fn stored(def: &MeterDef, base: i64, offset: i64) -> bool {
    (def.min..=def.base_cap).contains(&base) && base.checked_add(offset).is_some()
}

pub fn current(def: &MeterDef, live: &MeterLive) -> Result<i64, Reason> {
    let sum = live
        .base
        .checked_add(live.offset)
        .ok_or(Reason::OutOfRange)?;
    Ok(sum.clamp(def.min, def.max))
}

/// `Ok(true)` means the stored pair changed and the caller should track
/// `StateKey::ActorMeter`. A clamp that leaves the pair alone is `Ok(false)`.
/// The live record is assigned only after `stored` accepts the next pair.
pub fn write(def: &MeterDef, live: &mut MeterLive, write: MeterWrite) -> Result<bool, Reason> {
    let (next_base, next_offset) = match write {
        MeterWrite::Base(delta) => {
            let sum = live.base.checked_add(delta).ok_or(Reason::OutOfRange)?;
            (sum.clamp(def.min, def.base_cap), live.offset)
        }
        MeterWrite::Offset(delta) => {
            let next = live.offset.checked_add(delta).ok_or(Reason::OutOfRange)?;
            (live.base, next)
        }
        MeterWrite::Snap => (live.base, 0),
    };
    if !stored(def, next_base, next_offset) {
        return Err(Reason::OutOfRange);
    }
    let changed = next_base != live.base || next_offset != live.offset;
    live.base = next_base;
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
        }
    }

    #[test]
    fn stored_accepts_a_sum_past_max_and_current_clamps() {
        let def = grit();
        assert!(stored(&def, 100, 20));
        let live = MeterLive {
            base: 100,
            offset: 20,
        };
        assert_eq!(current(&def, &live).unwrap(), 110);
    }

    #[test]
    fn stored_rejects_a_sum_that_does_not_fit() {
        assert!(!stored(&grit(), 3, i64::MAX));
    }
}
