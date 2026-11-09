//! The stories' components, keyed by story id. The pack's stories all exist; the save holds
//! only what is on them.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::id::{EntityRef, StoryId};
use super::meter::{self, Meter};
use super::{change, flag, nested_map, Writer};
use crate::canon::Canon;
use crate::Reason;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StoryComponents {
    /// The meters each story file declares.
    #[serde(with = "nested_map")]
    pub meters: BTreeMap<(StoryId, String), Meter>,
    #[serde(with = "nested_map::lists")]
    pub flags: BTreeSet<(StoryId, String)>,
}

impl StoryComponents {
    /// The current value of meter `key` on `story`, or `UNKNOWN_ID` when it does not carry it.
    pub fn meter_value(&self, canon: &Canon, story: &StoryId, key: &str) -> Result<i64, Reason> {
        let def = canon.meter_def(EntityRef::Story(story), key);
        meter::read(def, nested_map::get(&self.meters, story, key))
    }

    pub fn has_flag(&self, story: &StoryId, key: &str) -> bool {
        nested_map::contains(&self.flags, story, key)
    }
}

impl Writer<'_, StoryComponents> {
    /// Move `story`'s meter `key` by `delta` inside its bounds.
    pub fn shift_meter(
        &mut self,
        canon: &Canon,
        story: &StoryId,
        key: &str,
        delta: i64,
    ) -> Result<(), Reason> {
        let Writer { part, transaction } = self;
        let def = canon
            .meter_def(EntityRef::Story(story), key)
            .ok_or(Reason::UnknownId)?;
        let held = nested_map::get_mut(&mut part.meters, story, key).ok_or(Reason::UnknownId)?;
        let watched = change::Key::StoryMeter(story.to_string(), key.to_string());
        meter::shift(held, transaction, def, Ok(0), delta, watched)
    }

    /// Give `story` meter `key` as `meter` says, `None` taking it off, outside a round's rules.
    pub fn set_meter(
        &mut self,
        canon: &Canon,
        story: &StoryId,
        key: &str,
        meter: Option<Meter>,
    ) -> Result<(), Reason> {
        let Writer { part, transaction } = self;
        let def = canon.meter_def(EntityRef::Story(story), key);
        let entry = (story.clone(), key.to_string());
        let watched = change::Key::StoryMeter(story.to_string(), key.to_string());
        meter::set(
            &mut part.meters,
            transaction,
            entry,
            meter,
            def,
            Ok(0),
            watched,
        )
    }

    pub fn set_flag(&mut self, story: &StoryId, key: &str, on: bool) -> Result<(), Reason> {
        let Writer { part, transaction } = self;
        let entry = (story.clone(), key.to_string());
        let watched = change::Key::StoryFlag(story.to_string(), key.to_string());
        flag::set(&mut part.flags, transaction, entry, on, watched)
    }
}
