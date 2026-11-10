//! The places' components, keyed by place id. The pack's places all exist; the save holds
//! only what is on them.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::id::{EntityRef, ExitId, PlaceId};
use super::meter::{self, Meter};
use super::{change, flag, item, nested_map, undo, Writer};
use crate::canon::Canon;
use crate::Reason;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PlaceComponents {
    #[serde(with = "nested_map")]
    pub items: undo::Store<BTreeMap<(PlaceId, String), u32>>,
    /// The meters each place file declares.
    #[serde(with = "nested_map")]
    pub meters: undo::Store<BTreeMap<(PlaceId, String), Meter>>,
    #[serde(with = "nested_map::lists")]
    pub flags: undo::Store<BTreeSet<(PlaceId, String)>>,
    /// The name of each exit whose door is open, by its place.
    #[serde(with = "nested_map::lists")]
    pub open_exits: undo::Store<BTreeSet<(PlaceId, String)>>,
}

impl PlaceComponents {
    /// The current value of meter `key` on `place`, or `UNKNOWN_ID` when it does not carry it.
    pub fn meter_value(&self, canon: &Canon, place: &PlaceId, key: &str) -> Result<i64, Reason> {
        let def = canon.meter_def(EntityRef::Place(place), key);
        meter::read(def, nested_map::get(&self.meters, place, key))
    }

    /// How many of `item` lie in `place`.
    pub fn item_count(&self, place: &PlaceId, item: &str) -> u32 {
        nested_map::get(&self.items, place, item)
            .copied()
            .unwrap_or(0)
    }

    pub fn has_flag(&self, place: &PlaceId, key: &str) -> bool {
        nested_map::contains(&self.flags, place, key)
    }

    /// Whether the door of `exit` is open.
    pub fn is_exit_open(&self, exit: &ExitId) -> bool {
        nested_map::contains(&self.open_exits, &exit.place, &exit.exit)
    }

    /// Every store, as the log a rollback undoes and an accept clears.
    pub(super) fn logs_mut(&mut self) -> [&mut dyn undo::Log; 4] {
        // Every store, with no `..`: a store added later must be listed here to compile.
        let PlaceComponents {
            items,
            meters,
            flags,
            open_exits,
        } = self;
        [items, meters, flags, open_exits]
    }
}

impl Writer<'_, PlaceComponents> {
    /// Move the count of `id` lying in `place` as `item::shift_count` says. A place the pack does
    /// not define is `UNKNOWN_ID`.
    pub fn shift_item(
        &mut self,
        canon: &Canon,
        place: &PlaceId,
        id: &str,
        delta: i64,
        max: Option<i64>,
    ) -> Result<(), Reason> {
        let Writer {
            components,
            transaction,
        } = self;
        canon.places.get(place).ok_or(Reason::UnknownId)?;
        let entry = (place.clone(), id.to_string());
        let key = change::Key::PlaceItem(place.to_string(), id.to_string());
        let (next, change) = item::shift(components.items.get(&entry).copied(), delta, max, key)?;
        components.items.set(transaction, entry, next, [change])
    }

    /// Move `place`'s meter `key` by `delta` inside its bounds.
    pub fn shift_meter(
        &mut self,
        canon: &Canon,
        place: &PlaceId,
        key: &str,
        delta: i64,
    ) -> Result<(), Reason> {
        let Writer {
            components,
            transaction,
        } = self;
        let def = canon
            .meter_def(EntityRef::Place(place), key)
            .ok_or(Reason::UnknownId)?;
        let entry = (place.clone(), key.to_string());
        let watched = change::Key::PlaceMeter(place.to_string(), key.to_string());
        let (next, change) =
            meter::shift(components.meters.get(&entry), def, Ok(0), delta, watched)?;
        components
            .meters
            .set(transaction, entry, Some(next), [change])
    }

    /// Give `place` meter `key` as `meter` says, `None` taking it off, outside a round's rules.
    pub fn set_meter(
        &mut self,
        canon: &Canon,
        place: &PlaceId,
        key: &str,
        meter: Option<Meter>,
    ) -> Result<(), Reason> {
        let Writer {
            components,
            transaction,
        } = self;
        let def = canon.meter_def(EntityRef::Place(place), key);
        let entry = (place.clone(), key.to_string());
        let watched = change::Key::PlaceMeter(place.to_string(), key.to_string());
        let (next, change) = meter::next(components.meters.get(&entry), meter, def, Ok(0), watched);
        components.meters.set(transaction, entry, next, [change])
    }

    pub fn set_flag(&mut self, place: &PlaceId, key: &str, on: bool) -> Result<(), Reason> {
        let Writer {
            components,
            transaction,
        } = self;
        let entry = (place.clone(), key.to_string());
        let watched = change::Key::PlaceFlag(place.to_string(), key.to_string());
        let (next, change) = flag::next(on, watched);
        components.flags.set(transaction, entry, next, [change])
    }

    /// Open or shut the door of `exit`.
    pub fn set_exit_open(&mut self, exit: &ExitId, open: bool) -> Result<(), Reason> {
        let Writer {
            components,
            transaction,
        } = self;
        let entry = (exit.place.clone(), exit.exit.clone());
        let watched = change::Key::ExitOpen(exit.place.to_string(), exit.exit.clone());
        let (next, change) = flag::next(open, watched);
        components
            .open_exits
            .set(transaction, entry, next, [change])
    }
}
