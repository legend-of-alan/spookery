//! Change keys a commit watches, and their previous values.

use std::fmt;

pub(crate) use spookery_protocol::ChangeValue;

use crate::ecs::component::Meter;
use crate::ecs::entity;
use crate::Reason;

/// One budget identity: the component a commit changed, the entity that holds it, and the
/// id of the entry under that component, `None` when the entity holds one value of it. The
/// commit diff summary is this type's `Display`, sorted as text. It is not a path inside
/// `save.yaml`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Key {
    /// The component's `ID` or, for an actor's action state, the part of it that moved.
    pub component_id: &'static str,
    pub entity_id: entity::Id,
    /// The id the entity holds the value under, such as meter `vigor` or item `rope`.
    pub entry_id: Option<String>,
}

/// `<collection>_<component> <id> [<entry>]`, such as `actor_meter rowan vigor` or
/// `world_flag bells_rung`: the world has no id to spell.
impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let component = self.component_id;
        match &self.entity_id {
            entity::Id::Actor(id) => write!(f, "actor_{component} {id}")?,
            entity::Id::Place(id) => write!(f, "place_{component} {id}")?,
            entity::Id::Path(id) => write!(f, "path_{component} {id}")?,
            entity::Id::Story(id) => write!(f, "story_{component} {id}")?,
            entity::Id::Global => write!(f, "world_{component}")?,
        }
        match &self.entry_id {
            Some(entry_id) => write!(f, " {entry_id}"),
            None => Ok(()),
        }
    }
}

impl Key {
    /// The key of `component_id`'s `entry_id` on `entity_id`: `None` when the entity holds one
    /// value of it.
    pub(crate) fn new(
        component_id: &'static str,
        entity_id: entity::IdRef<'_>,
        entry_id: Option<&str>,
    ) -> Key {
        Key {
            component_id,
            entity_id: match entity_id {
                entity::IdRef::Actor(id) => entity::Id::Actor(id.clone()),
                entity::IdRef::Place(id) => entity::Id::Place(id.clone()),
                entity::IdRef::Path(id) => entity::Id::Path(id.clone()),
                entity::IdRef::Story(id) => entity::Id::Story(id.clone()),
                entity::IdRef::Global => entity::Id::Global,
            },
            entry_id: entry_id.map(str::to_string),
        }
    }
}

/// A key that changed, with the value before the change and the value after it: `None` when
/// the key was absent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Change {
    pub key: Key,
    pub before: Option<ChangeValue>,
    pub after: Option<ChangeValue>,
}

impl Change {
    pub(crate) fn new(key: Key, before: Option<ChangeValue>, after: Option<ChangeValue>) -> Change {
        Change { key, before, after }
    }
}

/// What a store's write reports: a completed change, or, for a meter, what the write replaced, so
/// the take can read the meter's change as rules read the meter.
#[derive(Clone, Debug)]
pub(crate) enum Report {
    Change(Change),
    /// A meter's write: the stored meter it replaced.
    Meter(Key, Option<Meter>),
    /// A modifier's write: what the actor's modifiers added to the meter before it.
    Modifiers(Key, Result<i64, Reason>),
}

impl Report {
    pub(crate) fn key(&self) -> &Key {
        match self {
            Self::Change(change) => &change.key,
            Self::Meter(key, _) | Self::Modifiers(key, _) => key,
        }
    }
}

impl From<Change> for Report {
    fn from(change: Change) -> Self {
        Self::Change(change)
    }
}
