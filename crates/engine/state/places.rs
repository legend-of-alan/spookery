//! The places' components, keyed by place id. The pack's places all exist; the save holds
//! only what is on them.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::id::{EntityRef, ExitId, PlaceId};
use super::meter::{self, Meter};
use super::nested_map;
use crate::canon::Canon;
use crate::Reason;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PlaceComponents {
    #[serde(with = "nested_map")]
    pub items: BTreeMap<(PlaceId, String), u32>,
    /// The meters each place file declares.
    #[serde(with = "nested_map")]
    pub meters: BTreeMap<(PlaceId, String), Meter>,
    #[serde(with = "nested_map::lists")]
    pub flags: BTreeSet<(PlaceId, String)>,
    /// The name of each exit whose door is open, by its place.
    #[serde(with = "nested_map::lists")]
    pub open_exits: BTreeSet<(PlaceId, String)>,
}

impl PlaceComponents {
    /// The current value of meter `key` on `place`, or `UNKNOWN_ID` when it does not carry it.
    pub fn meter_value(&self, canon: &Canon, place: &PlaceId, key: &str) -> Result<i64, Reason> {
        let def = canon.meter_def(EntityRef::Place(place), key);
        meter::read(def, nested_map::get(&self.meters, place, key))
    }

    /// How many of `item` lie in `place`.
    pub fn item_count(&self, place: &PlaceId, item: &str) -> u32 {
        nested_map::get(&self.items, place, item)
            .copied()
            .unwrap_or(0)
    }

    pub fn has_flag(&self, place: &PlaceId, key: &str) -> bool {
        nested_map::contains(&self.flags, place, key)
    }

    /// Whether the door of `exit` is open.
    pub fn is_exit_open(&self, exit: &ExitId) -> bool {
        nested_map::contains(&self.open_exits, &exit.place, &exit.exit)
    }
}
