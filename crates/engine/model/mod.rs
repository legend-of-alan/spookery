//! The live save: places, actors, clocks, and notes.

pub(crate) mod key;
pub(crate) mod meter;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use meter::MeterLive;

use crate::intent::{Target, Viewer};

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
    pub world_time: u64,
    pub last_round_at: String,
    pub pause: Pause,
    pub places: BTreeMap<String, PlaceLive>,
    pub actors: BTreeMap<String, ActorLive>,
    pub clocks: BTreeMap<String, Clock>,
    pub notes: Vec<Note>,
    /// World flags. Omitted from the save when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flags: Vec<String>,
}

impl State {
    pub fn meter_value(
        &self,
        canon: &crate::canon::Canon,
        actor_id: &str,
        key: &str,
    ) -> Result<i64, crate::Reason> {
        let def = canon.meters.get(key).ok_or(crate::Reason::UnknownId)?;
        let live = self
            .actors
            .get(actor_id)
            .ok_or(crate::Reason::UnknownId)?
            .meters()
            .get(key)
            .ok_or(crate::Reason::UnknownId)?;
        meter::current(def, live)
    }
}

/// Who is seated. The host's seats own it and hand it to each round; the save never holds it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Presence {
    pub players: u32,
    pub actors_present: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Paused,
    Running,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
// Emit order is the docks `save.yaml` pause mapping.
pub struct Pause {
    pub presence_clocks: RunState,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaceLive {
    pub items: BTreeMap<String, u32>,
    pub notes: Vec<String>,
    pub actors: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ActorLive {
    #[serde(rename = "npc")]
    Npc {
        place: String,
        flags: Vec<String>,
        meters: BTreeMap<String, MeterLive>,
        inventory: BTreeMap<String, u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        wielded: Option<String>,
        known_facts: Vec<String>,
        act: Act,
    },
    #[serde(rename = "pc")]
    Pc {
        place: String,
        flags: Vec<String>,
        meters: BTreeMap<String, MeterLive>,
        inventory: BTreeMap<String, u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        wielded: Option<String>,
        known_facts: Vec<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        visited: Vec<String>,
        act: Act,
    },
}

impl ActorLive {
    pub fn place(&self) -> &str {
        match self {
            ActorLive::Npc { place, .. } | ActorLive::Pc { place, .. } => place,
        }
    }

    pub fn place_mut(&mut self) -> &mut String {
        match self {
            ActorLive::Npc { place, .. } | ActorLive::Pc { place, .. } => place,
        }
    }

    pub fn act(&self) -> &Act {
        match self {
            ActorLive::Npc { act, .. } | ActorLive::Pc { act, .. } => act,
        }
    }

    pub fn act_mut(&mut self) -> &mut Act {
        match self {
            ActorLive::Npc { act, .. } | ActorLive::Pc { act, .. } => act,
        }
    }

    /// Busy means `busy_until` is still after this world minute.
    pub fn is_busy(&self, world_time: u64) -> bool {
        self.act()
            .busy_until
            .is_some_and(|until| until > world_time)
    }

    pub fn visited(&self) -> &[String] {
        match self {
            ActorLive::Pc { visited, .. } => visited,
            ActorLive::Npc { .. } => &[],
        }
    }

    pub(crate) fn visit(&mut self, place: &str) -> bool {
        let ActorLive::Pc { visited, .. } = self else {
            return false;
        };
        if visited.iter().any(|id| id == place) {
            return false;
        }
        visited.push(place.to_string());
        visited.sort();
        true
    }

    pub fn flags(&self) -> &[String] {
        match self {
            ActorLive::Npc { flags, .. } | ActorLive::Pc { flags, .. } => flags,
        }
    }

    pub fn flags_mut(&mut self) -> &mut Vec<String> {
        match self {
            ActorLive::Npc { flags, .. } | ActorLive::Pc { flags, .. } => flags,
        }
    }

    pub fn known_facts(&self) -> &[String] {
        match self {
            ActorLive::Npc { known_facts, .. } | ActorLive::Pc { known_facts, .. } => known_facts,
        }
    }

    pub fn known_facts_mut(&mut self) -> &mut Vec<String> {
        match self {
            ActorLive::Npc { known_facts, .. } | ActorLive::Pc { known_facts, .. } => known_facts,
        }
    }

    pub fn is_npc(&self) -> bool {
        matches!(self, ActorLive::Npc { .. })
    }

    pub fn is_pc(&self) -> bool {
        matches!(self, ActorLive::Pc { .. })
    }

    /// The seat this actor acts from, given its own id.
    pub fn viewer(&self, id: &str) -> Viewer {
        match self {
            ActorLive::Pc { .. } => Viewer::Player(id.to_string()),
            ActorLive::Npc { .. } => Viewer::Npc(id.to_string()),
        }
    }

    pub fn meters(&self) -> &BTreeMap<String, MeterLive> {
        match self {
            ActorLive::Npc { meters, .. } | ActorLive::Pc { meters, .. } => meters,
        }
    }

    pub fn meters_mut(&mut self) -> &mut BTreeMap<String, MeterLive> {
        match self {
            ActorLive::Npc { meters, .. } | ActorLive::Pc { meters, .. } => meters,
        }
    }

    pub fn inventory(&self) -> &BTreeMap<String, u32> {
        match self {
            ActorLive::Npc { inventory, .. } | ActorLive::Pc { inventory, .. } => inventory,
        }
    }

    pub fn inventory_mut(&mut self) -> &mut BTreeMap<String, u32> {
        match self {
            ActorLive::Npc { inventory, .. } | ActorLive::Pc { inventory, .. } => inventory,
        }
    }

    pub fn item_count(&self, id: &str) -> u32 {
        self.inventory().get(id).copied().unwrap_or(0)
    }

    pub fn wielded(&self) -> Option<&str> {
        match self {
            ActorLive::Npc { wielded, .. } | ActorLive::Pc { wielded, .. } => wielded.as_deref(),
        }
    }

    pub fn wielded_mut(&mut self) -> &mut Option<String> {
        match self {
            ActorLive::Npc { wielded, .. } | ActorLive::Pc { wielded, .. } => wielded,
        }
    }

    /// Drop the wielded id once the actor no longer holds it.
    pub(crate) fn release_wielded(&mut self) -> bool {
        let Some(id) = self.wielded().map(str::to_string) else {
            return false;
        };
        if self.item_count(&id) > 0 {
            return false;
        }
        *self.wielded_mut() = None;
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
    pub idle_acts_today: u32,
    pub live_acts_this_hour: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub busy_until: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<PendingAction>,
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
pub struct Clock {
    pub value: i64,
    pub max: i64,
    pub requires_presence: bool,
    pub owners: Vec<String>,
    pub paused: bool,
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

pub fn sort_unique(items: &mut Vec<String>) {
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
