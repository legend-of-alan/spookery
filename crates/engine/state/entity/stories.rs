//! The stories' components, keyed by story id. The pack's stories all exist; the save holds
//! only what is on them.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::state::component::flag;
use crate::state::component::meter::{self, Meter};
use crate::state::id::StoryId;
use crate::state::{change, entity, nested_map, undo, Reader, Writer};
use crate::Reason;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Stories {
    /// The meters each story file declares.
    #[serde(with = "nested_map")]
    pub meters: undo::Store<BTreeMap<(StoryId, String), Meter>>,
    #[serde(with = "nested_map::lists")]
    pub flags: undo::Store<BTreeSet<(StoryId, String)>>,
}

impl Stories {
    pub fn has_flag(&self, story: &StoryId, key: &str) -> bool {
        nested_map::contains(&self.flags, story, key)
    }

    /// Every store, as the log a rollback undoes and an accept clears.
    pub(in crate::state) fn logs_mut(&mut self) -> [&mut dyn undo::Log; 2] {
        // Every store, with no `..`: a store added later must be listed here to compile.
        let Stories { meters, flags } = self;
        [meters, flags]
    }
}

impl Reader<'_, Stories> {
    /// The current value of meter `key` on `story`, or `UNKNOWN_ID` when it does not carry it.
    pub fn meter_value(self, story: &StoryId, key: &str) -> Result<i64, Reason> {
        let def = self.canon.meter_def(entity::IdRef::Story(story), key);
        meter::read(def, nested_map::get(&self.meters, story, key))
    }
}

impl Writer<'_, Stories> {
    /// Move `story`'s meter `key` by `delta` inside its bounds.
    pub fn shift_meter(&mut self, story: &StoryId, key: &str, delta: i64) -> Result<(), Reason> {
        let (components, transaction) = (&mut *self.components, &mut *self.transaction);
        let def = self
            .canon
            .meter_def(entity::IdRef::Story(story), key)
            .ok_or(Reason::UnknownId)?;
        let entry = (story.clone(), key.to_string());
        let watched = change::Key::StoryMeter(story.to_string(), key.to_string());
        let (next, change) =
            meter::shift(components.meters.get(&entry), def, Ok(0), delta, watched)?;
        components
            .meters
            .set(transaction, entry, Some(next), [change])
    }

    /// Give `story` meter `key` as `meter` says, `None` taking it off, outside a round's rules.
    pub fn set_meter(
        &mut self,
        story: &StoryId,
        key: &str,
        meter: Option<Meter>,
    ) -> Result<(), Reason> {
        let (components, transaction) = (&mut *self.components, &mut *self.transaction);
        let def = self.canon.meter_def(entity::IdRef::Story(story), key);
        let entry = (story.clone(), key.to_string());
        let watched = change::Key::StoryMeter(story.to_string(), key.to_string());
        let (next, change) = meter::next(components.meters.get(&entry), meter, def, Ok(0), watched);
        components.meters.set(transaction, entry, next, [change])
    }

    /// Set or clear `story`'s flag `key`. A story the pack does not define is `UNKNOWN_ID`.
    pub fn set_flag(&mut self, story: &StoryId, key: &str, on: bool) -> Result<(), Reason> {
        let (components, transaction) = (&mut *self.components, &mut *self.transaction);
        self.canon.stories.get(story).ok_or(Reason::UnknownId)?;
        let entry = (story.clone(), key.to_string());
        let watched = change::Key::StoryFlag(story.to_string(), key.to_string());
        let (next, change) = flag::next(on, watched);
        components.flags.set(transaction, entry, next, [change])
    }
}
