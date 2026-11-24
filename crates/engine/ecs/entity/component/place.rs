//! Where each actor stands.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::ecs::change::Change;
use crate::ecs::component::{text_value, Component, Mut, Ref};
use crate::ecs::entity;
use crate::ecs::id::{PathId, PlaceId};
use crate::Reason;

/// Where an actor stands: in a place, or on a path while they walk it, in neither room. The
/// save writes a place as its id and a walk as `{path: <id>, to: <place>}`: a two-way path
/// alone cannot say which way they go.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum Location {
    Place(PlaceId),
    Path { path: PathId, to: PlaceId },
}

impl Location {
    /// The place they stand in, `None` while they walk.
    pub fn place_id(&self) -> Option<&PlaceId> {
        match self {
            Location::Place(id) => Some(id),
            Location::Path { .. } => None,
        }
    }
}

impl From<&str> for Location {
    fn from(place_id: &str) -> Self {
        Location::Place(place_id.into())
    }
}

/// A place's id, or `path:<id>` for a walker: what an `at` condition and a change name.
impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Location::Place(id) => write!(f, "{id}"),
            Location::Path { path, .. } => write!(f, "{}", entity::Id::Path(path.clone())),
        }
    }
}

/// Where an actor stands.
pub struct Place;

impl Component for Place {
    type Value = Location;
    const ID: &'static str = "place";
}

impl<'a> Ref<'a, entity::Actors, Place> {
    /// Where the actor stands.
    pub fn get(&self) -> Option<&'a Location> {
        self.store().get(self.id)
    }

    /// The place the actor stands in, `None` while they walk.
    pub fn place_id(&self) -> Option<&'a PlaceId> {
        self.get().and_then(Location::place_id)
    }
}

impl Mut<'_, entity::Actors, Place> {
    /// Stand the actor at `to`: in a place, or on a path while they walk.
    pub fn set(&mut self, to: impl Into<Location>) -> Result<(), Reason> {
        self.check_exists()?;
        let to = to.into();
        let key = self.key(None);
        let before = text_value(self.store().get(self.id).map(Location::to_string));
        let change = Change::new(key, before, text_value(Some(to.to_string())));
        let (id, (place, transaction)) = (self.id, self.store_mut());
        place.set(transaction, id, to, [change.into()])
    }
}
