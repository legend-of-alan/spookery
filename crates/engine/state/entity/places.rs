//! The places' components, keyed by place id. The pack's places all exist; the save holds
//! only what is on them.

use serde::{Deserialize, Serialize};

use crate::canon::Canon;
use crate::state::component::{self, Meter};
use crate::state::id::{ExitId, PlaceId};
use crate::state::{entity, nested_map, undo, Reader, Writer};
use crate::Reason;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Places {
    #[serde(with = "nested_map")]
    pub items: undo::Store<component::Map<PlaceId, component::Item>>,
    /// The meters each place file declares.
    #[serde(with = "nested_map")]
    pub meters: undo::Store<component::Map<PlaceId, Meter>>,
    #[serde(with = "nested_map::lists")]
    pub flags: undo::Store<component::Map<PlaceId, component::Flag>>,
    /// The name of each exit whose door is open, by its place.
    #[serde(with = "nested_map::lists")]
    pub open_exits: undo::Store<component::Map<PlaceId, component::OpenExit>>,
}

impl entity::Entities for Places {
    type Id = PlaceId;

    fn contains(&self, canon: &Canon, id: &PlaceId) -> bool {
        canon.places.contains_key(id)
    }
}

entity::has!(Places, component::Item, items);
entity::has!(Places, Meter, meters);
entity::has!(Places, component::Flag, flags);
entity::has!(Places, component::OpenExit, open_exits);

impl Places {
    /// Every store, as the log a rollback undoes and an accept clears.
    pub(in crate::state) fn logs_mut(&mut self) -> [&mut dyn undo::Log; 4] {
        // Every store, with no `..`: a store added later must be listed here to compile.
        let Places {
            items,
            meters,
            flags,
            open_exits,
        } = self;
        [items, meters, flags, open_exits]
    }
}

impl Reader<'_, Places> {
    /// Whether the door of `exit` is open.
    pub fn is_exit_open(self, exit: &ExitId) -> bool {
        (self.get::<component::OpenExit>(&exit.place)).contains(&exit.exit)
    }
}

impl Writer<'_, Places> {
    /// Open the door of `exit`, or shut it.
    pub fn set_exit_open(&mut self, exit: &ExitId, open: bool) -> Result<(), Reason> {
        (self.get_mut::<component::OpenExit>(&exit.place)).set(&exit.exit, open)
    }
}
