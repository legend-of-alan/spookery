//! The stories' components, keyed by story id. The pack's stories all exist; the save holds
//! only what is on them.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::id::StoryId;
use super::meter::Meter;
use super::nested_map;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StoryComponents {
    /// The meters each story file declares.
    #[serde(with = "nested_map")]
    pub meters: BTreeMap<(StoryId, String), Meter>,
    #[serde(with = "nested_map::lists")]
    pub flags: BTreeSet<(StoryId, String)>,
}
