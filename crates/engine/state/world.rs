//! The world's components: it is one entity, so its stores need no owner id.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::meter::{self, Meter};
use super::undo::LogCut;
use super::{change, flag, Store, Writer};
use crate::canon::Canon;
use crate::Reason;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WorldComponents {
    /// The meters `world/meters.yaml` declares.
    pub meters: Store<BTreeMap<String, Meter>>,
    pub flags: Store<BTreeSet<String>>,
}

impl WorldComponents {
    /// The current value of world meter `key`, or `UNKNOWN_ID` when the pack does not give it.
    pub fn meter_value(&self, canon: &Canon, key: &str) -> Result<i64, Reason> {
        meter::read(canon.world_meters.get(key), self.meters.get(key))
    }

    pub fn has_flag(&self, key: &str) -> bool {
        self.flags.contains(key)
    }

    /// Truncate every store's log as `cut` says.
    pub(super) fn truncate_logs(&mut self, cut: LogCut) {
        // Every store, with no `..`: a store added later must be truncated here to compile.
        let WorldComponents { meters, flags } = self;
        meters.truncate_log(cut);
        flags.truncate_log(cut);
    }
}

impl Writer<'_, WorldComponents> {
    /// Move world meter `key` by `delta` inside its bounds.
    pub fn shift_meter(&mut self, canon: &Canon, key: &str, delta: i64) -> Result<(), Reason> {
        let Writer { part, transaction } = self;
        let def = canon.world_meters.get(key).ok_or(Reason::UnknownId)?;
        let watched = change::Key::WorldMeter(key.to_string());
        let (next, change) = meter::shift(part.meters.get(key), def, Ok(0), delta, watched)?;
        part.meters
            .set(transaction, key.to_string(), Some(next), [change])
    }

    /// Give the world meter `key` as `meter` says, `None` taking it off, outside a round's rules.
    pub fn set_meter(
        &mut self,
        canon: &Canon,
        key: &str,
        meter: Option<Meter>,
    ) -> Result<(), Reason> {
        let Writer { part, transaction } = self;
        let def = canon.world_meters.get(key);
        let watched = change::Key::WorldMeter(key.to_string());
        let (next, change) = meter::set(part.meters.get(key), meter, def, Ok(0), watched);
        part.meters
            .set(transaction, key.to_string(), next, [change])
    }

    pub fn set_flag(&mut self, key: &str, on: bool) -> Result<(), Reason> {
        let Writer { part, transaction } = self;
        let watched = change::Key::WorldFlag(key.to_string());
        let (next, change) = flag::set(on, watched);
        part.flags.set(transaction, key.to_string(), next, [change])
    }
}
