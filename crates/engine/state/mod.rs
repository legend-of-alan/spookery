//! The live save: the clock, each kind's components, and the notes.

mod actors;
pub(crate) mod change;
mod flag;
pub(crate) mod id;
mod item;
pub(crate) mod meter;
pub(crate) mod modifier;
pub(crate) mod nested_map;
mod notes;
mod places;
mod stories;
mod transaction;
mod world;

use std::ops::Deref;

use serde::{Deserialize, Serialize};

#[cfg(any(test, feature = "testing"))]
pub use actors::Act;
pub(crate) use actors::ActCounter;
pub use actors::{ActorComponents, ActorKind, PendingAction};
use id::ActorId;
pub use notes::Note;
pub use places::PlaceComponents;
pub use stories::StoryComponents;
use transaction::Transaction;
pub use world::WorldComponents;

use crate::intent::Viewer;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub schema: String,
    pub world_id: String,
    pub pack_rev: String,
    /// Engine rules version. Missing means 0, which is not the current version.
    #[serde(default)]
    pub rules_rev: u64,
    pub state_rev: u64,
    pub round_index: u64,
    /// Ticks since the world began.
    pub world_time: u64,
    pub last_round_at: String,
    pub actors: ActorComponents,
    #[serde(default)]
    pub places: PlaceComponents,
    #[serde(default)]
    pub stories: StoryComponents,
    #[serde(default)]
    pub world: WorldComponents,
    pub notes: Vec<Note>,
    #[serde(skip)]
    transaction: Transaction,
}

/// One part of the save with the round's transaction, borrowed apart from the rest: the only way
/// to write that part. Reads go through `Deref`; there is no `DerefMut`, and only `State`'s
/// `*_mut` getters build one.
///
/// ```compile_fail
/// # fn bypass(state: &mut spookery_engine::State) {
/// *state.actors_mut() = Default::default();
/// # }
/// ```
///
/// ```compile_fail
/// # fn bypass(state: &mut spookery_engine::State) {
/// let kind = *state.actors().kind.values().next().unwrap();
/// state.actors().kind.insert("x".into(), kind);
/// # }
/// ```
pub struct Writer<'a, P> {
    part: &'a mut P,
    transaction: &'a mut Transaction,
}

impl<P> Deref for Writer<'_, P> {
    type Target = P;
    fn deref(&self) -> &P {
        self.part
    }
}

impl State {
    pub fn actors(&self) -> &ActorComponents {
        &self.actors
    }

    pub fn places(&self) -> &PlaceComponents {
        &self.places
    }

    pub fn stories(&self) -> &StoryComponents {
        &self.stories
    }

    pub fn world(&self) -> &WorldComponents {
        &self.world
    }

    pub fn notes(&self) -> &[Note] {
        &self.notes
    }

    pub fn actors_mut(&mut self) -> Writer<'_, ActorComponents> {
        Writer {
            part: &mut self.actors,
            transaction: &mut self.transaction,
        }
    }

    pub fn places_mut(&mut self) -> Writer<'_, PlaceComponents> {
        Writer {
            part: &mut self.places,
            transaction: &mut self.transaction,
        }
    }

    pub fn stories_mut(&mut self) -> Writer<'_, StoryComponents> {
        Writer {
            part: &mut self.stories,
            transaction: &mut self.transaction,
        }
    }

    pub fn world_mut(&mut self) -> Writer<'_, WorldComponents> {
        Writer {
            part: &mut self.world,
            transaction: &mut self.transaction,
        }
    }

    pub fn notes_mut(&mut self) -> Writer<'_, Vec<Note>> {
        Writer {
            part: &mut self.notes,
            transaction: &mut self.transaction,
        }
    }

    /// The seat actor `id` acts from.
    pub(crate) fn viewer_of(&self, id: &ActorId) -> Option<Viewer> {
        self.actors.kind.get(id).map(|kind| kind.viewer(id))
    }

    /// The actors in `place`, in id order.
    pub(crate) fn actors_at(&self, place: &str) -> Vec<ActorId> {
        self.actors
            .place
            .iter()
            .filter(|(_, at)| *at == place)
            .map(|(id, _)| id.clone())
            .collect()
    }
}

/// Who is seated. The host's seats own it and hand it to each round; the save never holds it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Presence {
    pub players: u32,
    pub actors_present: Vec<ActorId>,
}

/// The place and exit names in an exit's id, `exit:<place>.<exit>`: where a walker stands
/// until they arrive. `None` for any other id.
pub(crate) fn exit_parts(id: &str) -> Option<(&str, &str)> {
    let (place, exit) = id.strip_prefix("exit:")?.split_once('.')?;
    (!place.is_empty() && !exit.is_empty()).then_some((place, exit))
}

pub(crate) fn sort_unique<T: Ord>(items: &mut Vec<T>) {
    items.sort();
    items.dedup();
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryLine {
    pub round_index: u64,
    pub actor_id: ActorId,
    pub intent_id: String,
    pub fact_ids: Vec<String>,
    pub tags: Vec<String>,
    pub text: Option<String>,
}
