//! The places' components, keyed by place id. The pack's places all exist; the save holds
//! only what is on them.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::grouped;
use super::id::{ExitId, PlaceId};
use super::meter::Meter;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PlaceComponents {
    #[serde(with = "grouped")]
    pub items: BTreeMap<(PlaceId, String), u32>,
    /// The meters each place file declares.
    #[serde(with = "grouped")]
    pub meters: BTreeMap<(PlaceId, String), Meter>,
    #[serde(with = "grouped::lists")]
    pub flags: BTreeSet<(PlaceId, String)>,
    /// The name of each exit whose door is open, by its place.
    #[serde(with = "grouped::lists")]
    pub open_exits: BTreeSet<(PlaceId, String)>,
}

impl PlaceComponents {
    /// Whether the door of `exit` is open.
    pub fn is_exit_open(&self, exit: &ExitId) -> bool {
        (self.open_exits).contains(&(exit.place.clone(), exit.exit.clone()))
    }
}
