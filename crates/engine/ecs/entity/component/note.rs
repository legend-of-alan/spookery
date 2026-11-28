//! The notes left in the world, each under its id, and leaving one.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::ecs::change::{Change, ChangeValue};
use crate::ecs::component::{Component, Mut};
use crate::ecs::entity;
use crate::ecs::entity::globals;
use crate::ecs::id::{ActorId, PlaceId};
use crate::Reason;

/// A note: who it is for, where it lies, and what it says. The world holds it under its id.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Note {
    #[serde(rename = "to")]
    pub to_ids: BTreeSet<ActorId>,
    #[serde(rename = "place", default, skip_serializing_if = "Option::is_none")]
    pub place_id: Option<PlaceId>,
    pub text: String,
}

impl Note {
    /// Whether the reader reads this note: it is left to them, or it lies where they stand.
    pub(crate) fn is_read_by(&self, reader_id: &ActorId, reader_place_id: Option<&str>) -> bool {
        let note_place_id = self.place_id.as_ref().map(PlaceId::as_str);
        self.to_ids.contains(reader_id)
            || (note_place_id.is_some() && note_place_id == reader_place_id)
    }
}

impl Component for Note {
    type Value = Note;
    const ID: &'static str = "note";
}

impl Mut<'_, entity::Globals, Note> {
    /// Leave `note` under `note_id`. A note id is left once: `UNKNOWN_ID` again.
    pub fn leave(&mut self, note_id: &str, note: Note) -> Result<(), Reason> {
        let entry_id = (globals::Id, note_id.to_string());
        if self.store().contains_key(&entry_id) {
            return Err(Reason::UnknownId);
        }
        let left = Some(ChangeValue::Bool(true));
        let reports = [Change::new(self.key(Some(note_id)), None, left).into()];
        let (notes, transaction) = self.store_mut();
        notes.set(transaction, entry_id, Some(note), reports)
    }
}
