//! The words a pack line names its actor and its owner with: `subject:` and `of:`.

use crate::state::id::{ActorId, ExitId, PlaceId, StoryId};

/// The actor an arrive or leave watch names with `subject:`. Omitted, it is `any`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Subject {
    Any,
    Pc,
    Npc,
    Actor(String),
}

/// A grammatical object a pack line names, including but not limited to a direct object, an
/// indirect object or the object of a preposition. Today it is the value of `of:`. A resist's `subject:` (the actor who rolls to resist, not the
/// subject of the outer line) reuses this value set until pack lines have a parse tree. The
/// pack writes `world` for `Object::Global`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Object {
    Self_,
    Target,
    Here,
    Global,
    Place(PlaceId),
    Story(StoryId),
    /// An exit, by its place and its name: `exit:<place>.<exit>`.
    Exit(ExitId),
    Actor(ActorId),
}
