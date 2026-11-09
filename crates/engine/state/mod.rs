//! The live save: the clock, each kind's components, and the notes.

mod actors;
pub(crate) mod change;
pub(crate) mod grouped;
pub(crate) mod id;
pub(crate) mod meter;
pub(crate) mod modifier;
mod places;
mod stories;
mod world;

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

pub(crate) use actors::ActCounter;
pub use actors::{Act, ActorComponents, ActorKind, PendingAction};
use id::{ActorId, EntityId, PlaceId};
use meter::Meter;
pub use places::PlaceComponents;
pub use stories::StoryComponents;
pub use world::WorldComponents;

use crate::canon::OPEN;
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
}

impl State {
    /// The current value of meter `key` on `entity`, or `UNKNOWN_ID` when it does not carry it.
    pub fn meter_value(
        &self,
        canon: &crate::canon::Canon,
        entity: &EntityId,
        key: &str,
    ) -> Result<i64, crate::Reason> {
        let def = canon
            .meter_def(entity.as_ref(), key)
            .ok_or(crate::Reason::UnknownId)?;
        let meters = self.meters_of(entity);
        let live = meters.get(key).ok_or(crate::Reason::UnknownId)?;
        meter::current(def, live, self.modifier_sum(entity, key)?)
    }

    /// What `entity`'s modifiers add to its meter `key`. Only an actor holds modifiers.
    pub(crate) fn modifier_sum(&self, entity: &EntityId, key: &str) -> Result<i64, crate::Reason> {
        match entity {
            EntityId::Actor(id) => {
                let held = grouped::of(&self.actors.modifiers, id).map(|(_, held)| held);
                modifier::sum(held, key)
            }
            EntityId::World | EntityId::Place(_) | EntityId::Story(_) | EntityId::Exit(..) => Ok(0),
        }
    }

    /// The live meters `entity` carries, by id.
    pub(crate) fn meters_of(&self, entity: &EntityId) -> BTreeMap<&str, &Meter> {
        match entity {
            EntityId::Actor(id) => grouped::of(&self.actors.meters, id).collect(),
            EntityId::Place(id) => grouped::of(&self.places.meters, id).collect(),
            EntityId::Story(id) => grouped::of(&self.stories.meters, id).collect(),
            EntityId::Exit(..) => BTreeMap::new(),
            EntityId::World => self
                .world
                .meters
                .iter()
                .map(|(id, live)| (id.as_str(), live))
                .collect(),
        }
    }

    /// Live meter `key` on `entity`, to write.
    pub(crate) fn meter_mut(
        &mut self,
        entity: &EntityId,
        key: &str,
    ) -> Result<&mut Meter, crate::Reason> {
        let key = key.to_string();
        let live = match entity {
            EntityId::Actor(id) => self.actors.meters.get_mut(&(id.clone(), key)),
            EntityId::Place(id) => self.places.meters.get_mut(&(id.clone(), key)),
            EntityId::Story(id) => self.stories.meters.get_mut(&(id.clone(), key)),
            EntityId::Exit(..) => None,
            EntityId::World => self.world.meters.get_mut(&key),
        };
        live.ok_or(crate::Reason::UnknownId)
    }

    /// Whether `entity` has flag `key` set. An exit's `open` is its door in `open_exits`.
    pub(crate) fn has_flag(&self, entity: &EntityId, key: &str) -> bool {
        let key = key.to_string();
        match entity {
            EntityId::Actor(id) => self.actors.flags.contains(&(id.clone(), key)),
            EntityId::Place(id) => self.places.flags.contains(&(id.clone(), key)),
            EntityId::Story(id) => self.stories.flags.contains(&(id.clone(), key)),
            EntityId::Exit(place, exit) => {
                key == OPEN
                    && self
                        .places
                        .open_exits
                        .contains(&(place.clone(), exit.clone()))
            }
            EntityId::World => self.world.flags.contains(&key),
        }
    }

    /// Set or clear `entity`'s flag `key`, and say whether it changed.
    pub(crate) fn set_flag(
        &mut self,
        entity: &EntityId,
        key: &str,
        value: bool,
    ) -> Result<bool, crate::Reason> {
        let key = key.to_string();
        Ok(match entity {
            EntityId::Actor(id) if !self.actors.kind.contains_key(id) => {
                return Err(crate::Reason::UnknownId)
            }
            EntityId::Actor(id) => turn(&mut self.actors.flags, (id.clone(), key), value),
            EntityId::Place(id) => turn(&mut self.places.flags, (id.clone(), key), value),
            EntityId::Story(id) => turn(&mut self.stories.flags, (id.clone(), key), value),
            EntityId::Exit(place, exit) => turn(
                &mut self.places.open_exits,
                (place.clone(), exit.clone()),
                value,
            ),
            EntityId::World => turn(&mut self.world.flags, key, value),
        })
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

/// Put `entry` in `set` or take it out, and say whether that changed the set.
fn turn<K: Ord>(set: &mut BTreeSet<K>, entry: K, on: bool) -> bool {
    if on {
        set.insert(entry)
    } else {
        set.remove(&entry)
    }
}

/// Who is seated. The host's seats own it and hand it to each round; the save never holds it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Presence {
    pub players: u32,
    pub actors_present: Vec<ActorId>,
}

/// Add `delta` to the count under `key`. Removing from a count of 0 is `MISSING_ITEM`.
/// A shortfall from a positive count is `OUT_OF_RANGE`. A positive `max` caps the result.
pub(crate) fn shift_count<K: Ord>(
    counts: &mut BTreeMap<K, u32>,
    key: K,
    delta: i64,
    max: Option<i64>,
) -> Result<bool, crate::Reason> {
    let current = i64::from(counts.get(&key).copied().unwrap_or(0));
    let mut next = current
        .checked_add(delta)
        .ok_or(crate::Reason::OutOfRange)?;
    if next < 0 {
        if current == 0 {
            return Err(crate::Reason::MissingItem);
        }
        return Err(crate::Reason::OutOfRange);
    }
    if delta > 0 {
        if let Some(cap) = max {
            if cap < 0 {
                return Err(crate::Reason::OutOfRange);
            }
            if next > cap {
                next = cap;
            }
        }
    }
    let stored = u32::try_from(next).map_err(|_| crate::Reason::OutOfRange)?;
    let changed = stored != u32::try_from(current).unwrap_or(0);
    if stored == 0 {
        counts.remove(&key);
    } else {
        counts.insert(key, stored);
    }
    Ok(changed)
}

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
