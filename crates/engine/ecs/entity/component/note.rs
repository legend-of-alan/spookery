//! The notes left in the world, each under its id, and leaving one.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::ecs::change::{Change, ChangeValue};
use crate::ecs::component::{Component, Mut};
use crate::ecs::entity;
use crate::ecs::id::{ActorId, GlobalId, PlaceId};
use crate::Reason;

/// A note: who it is for, where it lies, and the facts it carries. The world holds it under its
/// id.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Note {
    pub to_ids: BTreeSet<ActorId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place_id: Option<PlaceId>,
    pub fact_ids: BTreeSet<String>,
    pub text: String,
}

impl Component for Note {
    type Value = Note;
    const ID: &'static str = "note";
}

impl Mut<'_, entity::Globals, Note> {
    /// Leave `note` under `note_id`. A note id is left once: `UNKNOWN_ID` again.
    pub fn leave(&mut self, note_id: &str, note: Note) -> Result<(), Reason> {
        let entry_id = (GlobalId, note_id.to_string());
        if self.store().contains_key(&entry_id) {
            return Err(Reason::UnknownId);
        }
        let left = Some(ChangeValue::Bool(true));
        let reports = [Change::new(self.key(Some(note_id)), None, left).into()];
        let (notes, transaction) = self.store_mut();
        notes.set(transaction, entry_id, Some(note), reports)
    }
}
