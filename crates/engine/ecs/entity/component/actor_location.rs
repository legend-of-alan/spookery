//! Where an actor stands: in a place, or on a path while they walk it.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::ecs::entity;
use crate::ecs::id::{PathId, PlaceId};

/// Where an actor stands: in a place, or on a path while they walk it, in neither room. The
/// save writes a place as its id and a walk as `{path: <id>, to: <place>}`: a two-way path
/// alone cannot say which way they go.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum ActorLocation {
    Place(PlaceId),
    Path { path: PathId, to: PlaceId },
}

impl ActorLocation {
    /// The place they stand in, `None` while they walk.
    pub fn place_id(&self) -> Option<&PlaceId> {
        match self {
            ActorLocation::Place(id) => Some(id),
            ActorLocation::Path { .. } => None,
        }
    }
}

impl From<&str> for ActorLocation {
    fn from(place_id: &str) -> Self {
        ActorLocation::Place(place_id.into())
    }
}

/// A place's id, or `path:<id>` for a walker: what an `at` condition and a change name.
impl fmt::Display for ActorLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ActorLocation::Place(id) => write!(f, "{id}"),
            ActorLocation::Path { path, .. } => write!(f, "{}", entity::Id::Path(path.clone())),
        }
    }
}
