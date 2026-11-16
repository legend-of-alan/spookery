//! The notes left in the world, and leaving one.

use serde::{Deserialize, Serialize};

use super::change::{self, Change, ChangeValue};
use super::entity;
use super::id::{ActorId, PlaceId};
use super::{sort_unique, undo, Writer};
use crate::Reason;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Note {
    pub id: String,
    pub to: Vec<ActorId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<PlaceId>,
    pub fact_ids: Vec<String>,
    pub text: String,
}

impl Note {
    /// The id a note's change key spells: notes are the world's, under their id.
    pub(crate) const ID: &'static str = "note";
}

impl undo::Undo for Vec<Note> {
    type Inverse = ();
    /// Take back the last note left: leaving one is the only change the notes take.
    fn undo(&mut self, (): ()) {
        self.pop();
    }
}

impl Writer<'_, undo::Store<Vec<Note>>> {
    /// Leave `note`, its recipients and facts in order. A note id is left once: `UNKNOWN_ID` again.
    pub fn leave(&mut self, mut note: Note) -> Result<(), Reason> {
        let (components, transaction) = (&mut *self.components, &mut *self.transaction);
        if components.iter().any(|held| held.id == note.id) {
            return Err(Reason::UnknownId);
        }
        sort_unique(&mut note.to);
        sort_unique(&mut note.fact_ids);
        let key = change::Key::new(Note::ID, entity::IdRef::Global, Some(&note.id));
        let change = Change::new(key, None, Some(ChangeValue::Bool(true)));
        let leave = |notes: &mut Vec<Note>| notes.push(note);
        components.record(transaction, leave, [change.into()])
    }
}
