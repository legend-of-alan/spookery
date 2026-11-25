//! The places' components, keyed by place id. The pack's places all exist; the save holds
//! only what is on them.

use serde::{Deserialize, Serialize};

use crate::canon::Canon;
use crate::ecs::component::{self, Meter};
use crate::ecs::id::PlaceId;
use crate::ecs::{entity, nested_map, undo};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Places {
    #[serde(with = "nested_map")]
    items: undo::Store<component::Map<PlaceId, component::Item>>,
    /// The meters each place file declares.
    #[serde(with = "nested_map")]
    meters: undo::Store<component::Map<PlaceId, Meter>>,
    #[serde(with = "nested_map::lists")]
    flags: undo::Store<component::Map<PlaceId, component::Flag>>,
}

impl entity::Collection for Places {
    type Id = PlaceId;

    fn contains(&self, canon: &Canon, id: &PlaceId) -> bool {
        canon.places.contains_key(id)
    }

    fn ids<'a>(&'a self, canon: &'a Canon) -> entity::query::Ids<'a, Self> {
        Box::new(canon.places.keys())
    }
}

entity::has!(Places, component::Item, items);
entity::has!(Places, Meter, meters);
entity::has!(Places, component::Flag, flags);

impl Places {
    /// Every store, as the history a rollback undoes and an accept clears.
    pub(in crate::ecs) fn history_mut(&mut self) -> [&mut dyn undo::History; 3] {
        let Places {
            items,
            meters,
            flags,
        } = self;
        [items, meters, flags]
    }
}
