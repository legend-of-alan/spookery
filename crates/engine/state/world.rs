//! The world's components: it is one entity, so its stores need no owner id.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::meter::Meter;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WorldComponents {
    /// The meters `world/meters.yaml` declares.
    pub meters: BTreeMap<String, Meter>,
    pub flags: BTreeSet<String>,
}
