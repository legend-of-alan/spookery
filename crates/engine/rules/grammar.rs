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

/// The owner a rule's `of:` names, and the actor a resist's `subject:` names. The pack writes
/// `world` for `Object::Global`.
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
