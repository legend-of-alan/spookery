//! The stories' components, keyed by story id. The pack's stories all exist; the save holds
//! only what is on them.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::grouped;
use super::id::StoryId;
use super::meter::Meter;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StoryComponents {
    /// The meters each story file declares.
    #[serde(with = "grouped")]
    pub meters: BTreeMap<(StoryId, String), Meter>,
    #[serde(with = "grouped::lists")]
    pub flags: BTreeSet<(StoryId, String)>,
}
