//! The world's components: it is one entity, so the save writes its stores with no owner id.

use serde::{Deserialize, Serialize};

use crate::canon::Canon;
use crate::state::component::{self, Meter};
use crate::state::id::GlobalId;
use crate::state::{entity, nested_map, undo};

/// The world's collection: what the one world holds. The wire and the save still say `world`
/// (`world_flag`, `world_meter`, the `world:` key, and `entity::Id::Global`, which serializes as
/// `world`), and those names do not change.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Globals {
    /// The meters `world/meters.yaml` declares.
    #[serde(with = "nested_map::flat")]
    pub(in crate::state) meters: undo::Store<component::Map<GlobalId, Meter>>,
    #[serde(with = "nested_map::flat::lists")]
    pub(in crate::state) flags: undo::Store<component::Map<GlobalId, component::Flag>>,
}

impl entity::Entities for Globals {
    type Id = GlobalId;

    fn contains(&self, _: &Canon, _: &GlobalId) -> bool {
        true
    }

    fn ids<'a>(&'a self, _: &'a Canon) -> entity::query::Ids<'a, Self> {
        Box::new(std::iter::once(&GlobalId))
    }
}

entity::has!(Globals, Meter, meters);
entity::has!(Globals, component::Flag, flags);

impl Globals {
    /// Every store, as the log a rollback undoes and an accept clears.
    pub(in crate::state) fn logs_mut(&mut self) -> [&mut dyn undo::Log; 2] {
        // Every store, with no `..`: a store added later must be listed here to compile.
        let Globals { meters, flags } = self;
        [meters, flags]
    }
}
