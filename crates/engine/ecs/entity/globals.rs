//! The world's components: it is one entity, so the save writes its stores with no parent id.

use serde::{Deserialize, Serialize};

use crate::canon::Canon;
use crate::ecs::component::{self, Meter, Note};
use crate::ecs::id::AsIdRef;
use crate::ecs::{entity, nested_map, undo};

/// The world's id. There is one world, so its id says nothing more.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Id;

impl AsIdRef for Id {
    fn as_id_ref(&self) -> entity::IdRef<'_> {
        entity::IdRef::Global
    }
}

/// The world's collection: what the one world holds. The save keeps it under `globals:`. The
/// wire and the journal still say `world` (`world_flag`, `world_meter`, and `entity::Id::Global`,
/// which serializes as `world`), and those names do not change.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Globals {
    /// The meters `world/meters.yaml` declares.
    #[serde(with = "nested_map::flat")]
    meters: undo::Store<component::Map<Id, Meter>>,
    #[serde(with = "nested_map::flat::lists")]
    flags: undo::Store<component::Map<Id, component::Flag>>,
    /// The notes left in the world, by id.
    #[serde(with = "nested_map::flat")]
    notes: undo::Store<component::Map<Id, Note>>,
}

impl entity::Collection for Globals {
    type Id = Id;

    fn contains(&self, _: &Canon, _: &Id) -> bool {
        true
    }

    fn ids<'a>(&'a self, _: &'a Canon) -> entity::query::Ids<'a, Self> {
        Box::new(std::iter::once(&Id))
    }
}

entity::has!(Globals, Meter, meters);
entity::has!(Globals, component::Flag, flags);
entity::has!(Globals, Note, notes);

impl Globals {
    /// Every store, as the history a rollback undoes and an accept clears.
    pub(in crate::ecs) fn history_mut(&mut self) -> [&mut dyn undo::History; 3] {
        let Globals {
            meters,
            flags,
            notes,
        } = self;
        [meters, flags, notes]
    }
}
