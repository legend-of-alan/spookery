//! The stories' components, keyed by story id. The pack's stories all exist; the save holds
//! only what is on them.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::id::{EntityRef, StoryId};
use super::meter::{self, Meter};
use super::nested_map;
use crate::canon::Canon;
use crate::Reason;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StoryComponents {
    /// The meters each story file declares.
    #[serde(with = "nested_map")]
    pub meters: BTreeMap<(StoryId, String), Meter>,
    #[serde(with = "nested_map::lists")]
    pub flags: BTreeSet<(StoryId, String)>,
}

impl StoryComponents {
    /// The current value of meter `key` on `story`, or `UNKNOWN_ID` when it does not carry it.
    pub fn meter_value(&self, canon: &Canon, story: &StoryId, key: &str) -> Result<i64, Reason> {
        let def = canon.meter_def(EntityRef::Story(story), key);
        meter::read(def, nested_map::get(&self.meters, story, key))
    }

    pub fn has_flag(&self, story: &StoryId, key: &str) -> bool {
        nested_map::contains(&self.flags, story, key)
    }
}
