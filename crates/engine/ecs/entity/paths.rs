//! The paths' components, keyed by path id. The pack's paths all exist; the save holds only
//! the flags set on them, the one a door is: `open`. No pack declares a path meter yet, so
//! their store stays empty and out of the save.

use serde::{Deserialize, Serialize};

use crate::canon::Canon;
use crate::ecs::component::{self, Meter};
use crate::ecs::id::PathId;
use crate::ecs::{entity, nested_map, undo};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Paths {
    #[serde(with = "nested_map::lists")]
    flags: undo::Store<component::Map<PathId, component::Flag>>,
    #[serde(skip)]
    meters: undo::Store<component::Map<PathId, Meter>>,
}

impl entity::Collection for Paths {
    type Id = PathId;

    fn contains(&self, canon: &Canon, id: &PathId) -> bool {
        canon.paths.get(id).is_some()
    }

    fn ids<'a>(&'a self, canon: &'a Canon) -> entity::query::Ids<'a, Self> {
        Box::new(canon.paths.ids())
    }
}

entity::has!(Paths, component::Flag, flags);
entity::has!(Paths, Meter, meters);

impl Paths {
    /// Every store, as the history a rollback undoes and an accept clears.
    pub(in crate::ecs) fn history_mut(&mut self) -> [&mut dyn undo::History; 2] {
        let Paths { flags, meters } = self;
        [flags, meters]
    }
}
