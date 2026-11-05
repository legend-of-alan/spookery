//! The live save: places, actors, stories, world meters, and notes.

pub(crate) mod key;
pub(crate) mod meter;
pub(crate) mod modifier;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use key::StateKey;
use meter::MeterLive;
use modifier::ModifierLive;

use crate::intent::{Target, Viewer};
use crate::rules::effect::Holder;

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
    pub places: BTreeMap<String, PlaceLive>,
    pub actors: BTreeMap<String, ActorLive>,
    /// The meters and flags each story holds. Omitted when empty.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub stories: BTreeMap<String, StoryLive>,
    /// The meters `world/meters.yaml` declares, held once by the world. Omitted when empty.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub world_meters: BTreeMap<String, MeterLive>,
    pub notes: Vec<Note>,
    /// World flags. Omitted from the save when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flags: Vec<String>,
}

impl State {
    /// The current value of meter `key` on `holder`, or `UNKNOWN_ID` when it does not carry it.
    pub fn meter_value(
        &self,
        canon: &crate::canon::Canon,
        holder: &Holder,
        key: &str,
    ) -> Result<i64, crate::Reason> {
        let def = canon
            .meter_def(holder, key)
            .ok_or(crate::Reason::UnknownId)?;
        let live = self
            .meters_of(holder)
            .and_then(|meters| meters.get(key))
            .ok_or(crate::Reason::UnknownId)?;
        meter::current(def, live, self.modifier_sum(holder, key)?)
    }

    /// What `holder`'s modifiers add to its meter `key`. Only an actor holds modifiers.
    pub(crate) fn modifier_sum(&self, holder: &Holder, key: &str) -> Result<i64, crate::Reason> {
        match holder {
            Holder::Actor(id) => self
                .actors
                .get(id)
                .map_or(Ok(0), |actor| modifier::sum(&actor.modifiers, key)),
            Holder::World | Holder::Place(_) | Holder::Story(_) => Ok(0),
        }
    }

    /// The live meters `holder` carries.
    pub(crate) fn meters_of(&self, holder: &Holder) -> Option<&BTreeMap<String, MeterLive>> {
        match holder {
            Holder::Actor(id) => self.actors.get(id).map(|actor| &actor.meters),
            Holder::Place(id) => self.places.get(id).map(|place| &place.meters),
            Holder::Story(id) => self.stories.get(id).map(|story| &story.meters),
            Holder::World => Some(&self.world_meters),
        }
    }

    /// The live meters `holder` carries, to write.
    pub(crate) fn meters_mut(
        &mut self,
        holder: &Holder,
    ) -> Result<&mut BTreeMap<String, MeterLive>, crate::Reason> {
        match holder {
            Holder::Actor(id) => Ok(&mut self.actor_mut(id)?.meters),
            Holder::Place(id) => Ok(&mut self
                .places
                .get_mut(id)
                .ok_or(crate::Reason::UnknownId)?
                .meters),
            Holder::Story(id) => Ok(&mut self
                .stories
                .get_mut(id)
                .ok_or(crate::Reason::UnknownId)?
                .meters),
            Holder::World => Ok(&mut self.world_meters),
        }
    }

    /// Whether `holder` has flag `key` set.
    pub(crate) fn has_flag(&self, holder: &Holder, key: &str) -> bool {
        let flags = match holder {
            Holder::Actor(id) => self.actors.get(id).map(|actor| &actor.flags),
            Holder::Place(id) => self.places.get(id).map(|place| &place.flags),
            Holder::Story(id) => self.stories.get(id).map(|story| &story.flags),
            Holder::World => Some(&self.flags),
        };
        flags.is_some_and(|flags| flags.iter().any(|flag| flag == key))
    }

    /// The flags `holder` has set, to write. A place or story gets a live record on its first.
    pub(crate) fn flags_mut(&mut self, holder: &Holder) -> Result<&mut Vec<String>, crate::Reason> {
        match holder {
            Holder::Actor(id) => Ok(&mut self.actor_mut(id)?.flags),
            Holder::Place(id) => Ok(&mut self.places.entry(id.clone()).or_default().flags),
            Holder::Story(id) => Ok(&mut self.stories.entry(id.clone()).or_default().flags),
            Holder::World => Ok(&mut self.flags),
        }
    }

    /// Actor `id`'s live record to write, or `UNKNOWN_ID`.
    pub(crate) fn actor_mut(&mut self, id: &str) -> Result<&mut ActorLive, crate::Reason> {
        self.actors.get_mut(id).ok_or(crate::Reason::UnknownId)
    }

    /// The seat actor `id` acts from.
    pub(crate) fn viewer_of(&self, id: &str) -> Option<Viewer> {
        self.actors.get(id).map(|actor| actor.viewer(id))
    }

    /// The actors in `place`, in id order.
    pub(crate) fn actors_at(&self, place: &str) -> Vec<String> {
        self.actors
            .iter()
            .filter(|(_, actor)| actor.place == place)
            .map(|(id, _)| id.clone())
            .collect()
    }
}

/// Who is seated. The host's seats own it and hand it to each round; the save never holds it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Presence {
    pub players: u32,
    pub actors_present: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaceLive {
    pub items: BTreeMap<String, u32>,
    /// The meters the place file declares. Omitted when empty.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub meters: BTreeMap<String, MeterLive>,
    /// The flags set on this place. Omitted when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flags: Vec<String>,
}

/// The meters a story file declares and the flags set on that story.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoryLive {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub meters: BTreeMap<String, MeterLive>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flags: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActorLive {
    pub kind: ActorKind,
    pub place: String,
    pub flags: Vec<String>,
    pub meters: BTreeMap<String, MeterLive>,
    /// The modifiers this actor holds, by id. Omitted when empty.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub modifiers: BTreeMap<String, ModifierLive>,
    pub inventory: BTreeMap<String, u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wielded: Option<String>,
    pub known_facts: Vec<String>,
    /// Each place this actor has stood at the end of a round, with the names of the exits
    /// they saw there then. Omitted when empty.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub visited: BTreeMap<String, Vec<String>>,
    pub act: Act,
}

/// Whether a live actor is a PC or an NPC.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorKind {
    Npc,
    Pc,
}

impl ActorLive {
    /// Busy means `busy_until` is still after this tick.
    pub fn is_busy(&self, world_time: u64) -> bool {
        self.act.busy_until.is_some_and(|until| until > world_time)
    }

    pub fn is_npc(&self) -> bool {
        self.kind == ActorKind::Npc
    }

    pub fn is_pc(&self) -> bool {
        self.kind == ActorKind::Pc
    }

    /// The seat this actor acts from, given its own id.
    pub fn viewer(&self, id: &str) -> Viewer {
        match self.kind {
            ActorKind::Pc => Viewer::Player(id.to_string()),
            ActorKind::Npc => Viewer::Npc(id.to_string()),
        }
    }

    pub fn item_count(&self, id: &str) -> u32 {
        self.inventory.get(id).copied().unwrap_or(0)
    }

    /// Drop the wielded id once the actor no longer holds it.
    pub(crate) fn release_wielded(&mut self) -> bool {
        let Some(id) = self.wielded.clone() else {
            return false;
        };
        if self.item_count(&id) > 0 {
            return false;
        }
        self.wielded = None;
        true
    }
}

/// Add `delta` to a count. Removing from a count of 0 is `MISSING_ITEM`.
/// A shortfall from a positive count is `OUT_OF_RANGE`. A positive `max` caps the result.
pub(crate) fn shift_count(
    counts: &mut BTreeMap<String, u32>,
    id: &str,
    delta: i64,
    max: Option<i64>,
) -> Result<bool, crate::Reason> {
    let current = i64::from(counts.get(id).copied().unwrap_or(0));
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
        counts.remove(id);
    } else {
        counts.insert(id.to_string(), stored);
    }
    Ok(changed)
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// How many times an actor has acted, and the journey they are still on.
pub struct Act {
    pub last_act_round: u64,
    pub idle_acts_this_window: u32,
    pub live_acts_this_window: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub busy_until: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<PendingAction>,
}

/// One of the `Act` counts the end of its act window sets back to 0.
#[derive(Clone, Copy)]
pub(crate) enum ActCounter {
    IdleActsThisWindow,
    LiveActsThisWindow,
}

impl ActCounter {
    /// The key this count is tracked under for actor `id`.
    pub(crate) fn key(self, id: String) -> StateKey {
        match self {
            ActCounter::IdleActsThisWindow => StateKey::IdleActsThisWindow(id),
            ActCounter::LiveActsThisWindow => StateKey::LiveActsThisWindow(id),
        }
    }

    /// This count on `act`, to write.
    pub(crate) fn count_mut(self, act: &mut Act) -> &mut u32 {
        match self {
            ActCounter::IdleActsThisWindow => &mut act.idle_acts_this_window,
            ActCounter::LiveActsThisWindow => &mut act.live_acts_this_window,
        }
    }
}

/// Work that finishes when `busy_until` is reached.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingAction {
    pub verb: String,
    pub source: crate::intent::Source,
    pub kind: PendingKind,
}

/// How a timed action finishes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum PendingKind {
    /// A journey: the place it ends in, through the exit it took.
    Arrive { place: String, via: String },
    /// Any other timed verb, with the target it was queued against and the rule it started with.
    Finish {
        #[serde(default, skip_serializing_if = "Target::is_empty")]
        target: Target,
        rule: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Note {
    pub id: String,
    pub to: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<String>,
    pub fact_ids: Vec<String>,
    pub text: String,
}

pub(crate) fn sort_unique(items: &mut Vec<String>) {
    items.sort();
    items.dedup();
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryLine {
    pub round_index: u64,
    pub actor_id: String,
    pub intent_id: String,
    pub fact_ids: Vec<String>,
    pub tags: Vec<String>,
    pub text: Option<String>,
}
