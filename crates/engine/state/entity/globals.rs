//! The world's components: it is one entity, so its stores need no owner id.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::canon::Canon;
use crate::state::component::flag;
use crate::state::component::meter::{self, Meter};
use crate::state::{change, undo, Writer};
use crate::Reason;

/// The world's collection: what the one world holds. The wire and the save still say `world`
/// (`world_flag`, `world_meter`, the `world:` key, and `entity::Id::Global`, which serializes as
/// `world`), and those names do not change.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Globals {
    /// The meters `world/meters.yaml` declares.
    pub meters: undo::Store<BTreeMap<String, Meter>>,
    pub flags: undo::Store<BTreeSet<String>>,
}

impl Globals {
    /// The current value of world meter `key`, or `UNKNOWN_ID` when the pack does not give it.
    pub fn meter_value(&self, canon: &Canon, key: &str) -> Result<i64, Reason> {
        meter::read(canon.world_meters.get(key), self.meters.get(key))
    }

    pub fn has_flag(&self, key: &str) -> bool {
        self.flags.contains(key)
    }

    /// Every store, as the log a rollback undoes and an accept clears.
    pub(in crate::state) fn logs_mut(&mut self) -> [&mut dyn undo::Log; 2] {
        // Every store, with no `..`: a store added later must be listed here to compile.
        let Globals { meters, flags } = self;
        [meters, flags]
    }
}

impl Writer<'_, Globals> {
    /// Move world meter `key` by `delta` inside its bounds.
    pub fn shift_meter(&mut self, canon: &Canon, key: &str, delta: i64) -> Result<(), Reason> {
        let (components, transaction) = (&mut *self.components, &mut *self.transaction);
        let def = canon.world_meters.get(key).ok_or(Reason::UnknownId)?;
        let watched = change::Key::WorldMeter(key.to_string());
        let (next, change) = meter::shift(components.meters.get(key), def, Ok(0), delta, watched)?;
        components
            .meters
            .set(transaction, key.to_string(), Some(next), [change])
    }

    /// Give the world meter `key` as `meter` says, `None` taking it off, outside a round's rules.
    pub fn set_meter(
        &mut self,
        canon: &Canon,
        key: &str,
        meter: Option<Meter>,
    ) -> Result<(), Reason> {
        let (components, transaction) = (&mut *self.components, &mut *self.transaction);
        let def = canon.world_meters.get(key);
        let watched = change::Key::WorldMeter(key.to_string());
        let (next, change) = meter::next(components.meters.get(key), meter, def, Ok(0), watched);
        components
            .meters
            .set(transaction, key.to_string(), next, [change])
    }

    pub fn set_flag(&mut self, key: &str, on: bool) -> Result<(), Reason> {
        let (components, transaction) = (&mut *self.components, &mut *self.transaction);
        let watched = change::Key::WorldFlag(key.to_string());
        let (next, change) = flag::next(on, watched);
        components
            .flags
            .set(transaction, key.to_string(), next, [change])
    }
}
