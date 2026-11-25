//! Where an actor stands: in a place, or on a path while they walk it.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::canon::{Canon, Exit};
use crate::ecs::entity;
use crate::ecs::id::{PathId, PlaceId};

/// Where an actor stands: in a place, or on a path while they walk it, in neither room. The
/// save writes a place as its id and a walk as `{path: <id>, to: <place>}`: a two-way path
/// alone cannot say which way they go.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum ActorLocation {
    Place(PlaceId),
    Path {
        #[serde(rename = "path")]
        path_id: PathId,
        #[serde(rename = "to")]
        to_place_id: PlaceId,
    },
}

impl ActorLocation {
    /// The place they stand in, `None` while they walk.
    pub fn place_id(&self) -> Option<&PlaceId> {
        match self {
            ActorLocation::Place(id) => Some(id),
            ActorLocation::Path { .. } => None,
        }
    }

    /// They are on a path, in neither room.
    pub(crate) fn is_walking(&self) -> bool {
        matches!(self, ActorLocation::Path { .. })
    }

    /// The exit a walker stands on: their path, walked toward where they head. `None` in a
    /// place, or when the pack has no such path toward that place.
    pub(crate) fn find_walked_exit<'c>(&self, canon: &'c Canon) -> Option<Exit<'c>> {
        let ActorLocation::Path {
            path_id,
            to_place_id,
        } = self
        else {
            return None;
        };
        canon.paths.get(path_id)?.exit_to(to_place_id.as_str())
    }

    /// Path `path_id` as an exit from their place. A walker is still on their exit when the walk
    /// finishes, so for a walker it is that exit, whatever `path_id` says.
    pub(crate) fn find_exit<'c>(&self, canon: &'c Canon, path_id: &PathId) -> Option<Exit<'c>> {
        match self {
            ActorLocation::Place(from_place_id) => {
                canon.paths.get(path_id)?.exit_from(from_place_id.as_str())
            }
            ActorLocation::Path { .. } => self.find_walked_exit(canon),
        }
    }

    /// The pack defines where they stand: their place, or their path walked toward where they
    /// head.
    pub(crate) fn is_known(&self, canon: &Canon) -> bool {
        match self {
            ActorLocation::Place(id) => canon.places.contains_key(id),
            ActorLocation::Path { .. } => self.find_walked_exit(canon).is_some(),
        }
    }

    /// The name a player reads for where they stand: their place's, or for a walker the name of
    /// the exit they walk, which follows the way they head. `None` when the pack has neither.
    pub(crate) fn find_name(&self, canon: &Canon) -> Option<String> {
        match self {
            ActorLocation::Place(id) => canon.places.get(id).map(|place| place.name.clone()),
            ActorLocation::Path { .. } => {
                (self.find_walked_exit(canon)).map(|exit| exit.name(canon).to_string())
            }
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
            ActorLocation::Path { path_id, .. } => {
                write!(f, "{}", entity::Id::Path(path_id.clone()))
            }
        }
    }
}
