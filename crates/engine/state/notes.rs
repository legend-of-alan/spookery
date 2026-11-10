//! The notes left in the world, and leaving one.

use serde::{Deserialize, Serialize};

use super::change::{self, Change};
use super::id::{ActorId, PlaceId};
use super::undo::{Store, Undo};
use super::{sort_unique, Writer};
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

impl Undo for Vec<Note> {
    type Inverse = ();
    /// Take back the last note left: leaving one is the only change the notes take.
    fn undo(&mut self, (): ()) {
        self.pop();
    }
}

impl Writer<'_, Store<Vec<Note>>> {
    /// Leave `note`, its recipients and facts in order. A note id is left once: `UNKNOWN_ID` again.
    pub fn leave(&mut self, mut note: Note) -> Result<(), Reason> {
        let Writer { part, transaction } = self;
        if part.iter().any(|held| held.id == note.id) {
            return Err(Reason::UnknownId);
        }
        sort_unique(&mut note.to);
        sort_unique(&mut note.fact_ids);
        let key = change::Key::Note(note.id.clone());
        let change = Change::new(key, change::Value::Empty, change::Value::Bool(true));
        let leave = |notes: &mut Vec<Note>| notes.push(note);
        part.record(transaction, leave, [change])
    }
}
