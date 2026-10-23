//! The live save: places, actors, clocks, and messages.

pub(crate) mod key;
pub(crate) mod meter;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub use crate::map::ExitView;
pub use crate::perceive::{ClockView, ItemStack, KnownFact, MeterView, View};
pub use meter::MeterLive;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Now {
    pub schema: String,
    pub shard_id: String,
    pub pack_rev: String,
    /// Engine rules version. Missing means 0, which is not the current version.
    #[serde(default)]
    pub rules_rev: u64,
    pub state_rev: u64,
    pub tick_index: u64,
    pub world_time: u64,
    pub last_tick_at: String,
    /// Live seats. Memory only: never written to or read from the save.
    #[serde(skip)]
    pub presence: Presence,
    pub pause: PauseView,
    pub places: BTreeMap<String, PlaceLive>,
    pub actors: BTreeMap<String, ActorLive>,
    pub clocks: BTreeMap<String, Clock>,
    pub messages: Vec<Message>,
    /// World flags. Omitted from the save when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flags: Vec<String>,
}

impl Now {
    pub fn viewer_of(&self, id: &str) -> Option<crate::intent::ViewerId> {
        let actor = self.actors.get(id)?;
        crate::intent::ViewerId::from_kind(actor.actor_kind(), id).ok()
    }

    pub(crate) fn seated_pcs(&self) -> Vec<String> {
        self.presence
            .actors_present
            .iter()
            .filter(|id| self.actors.get(id.as_str()).is_some_and(ActorLive::is_pc))
            .cloned()
            .collect()
    }

    pub fn meter_value(
        &self,
        canon: &crate::canon::Canon,
        actor_id: &str,
        key: &str,
    ) -> Result<i64, crate::error::Reason> {
        let def = canon
            .meters
            .get(key)
            .ok_or(crate::error::Reason::UnknownKey)?;
        let live = self
            .actors
            .get(actor_id)
            .ok_or(crate::error::Reason::UnknownKey)?
            .meters()
            .get(key)
            .ok_or(crate::error::Reason::UnknownKey)?;
        meter::current(def, live)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Presence {
    pub humans: u32,
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
pub struct PauseView {
    pub named_plot_clocks: RunState,
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
        act: ActCounters,
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
        act: ActCounters,
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

    pub fn act(&self) -> &ActCounters {
        match self {
            ActorLive::Npc { act, .. } | ActorLive::Pc { act, .. } => act,
        }
    }

    pub fn act_mut(&mut self) -> &mut ActCounters {
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

    pub fn actor_kind(&self) -> crate::intent::ActorKind {
        if self.is_pc() {
            crate::intent::ActorKind::Player
        } else {
            crate::intent::ActorKind::Npc
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

    pub fn count(&self, id: &str) -> u32 {
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
        if self.count(&id) > 0 {
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
) -> Result<bool, crate::error::Reason> {
    let current = i64::from(counts.get(id).copied().unwrap_or(0));
    let mut next = current
        .checked_add(delta)
        .ok_or(crate::error::Reason::OutOfRange)?;
    if next < 0 {
        if current == 0 {
            return Err(crate::error::Reason::MissingItem);
        }
        return Err(crate::error::Reason::OutOfRange);
    }
    if delta > 0 {
        if let Some(cap) = max {
            if cap < 0 {
                return Err(crate::error::Reason::OutOfRange);
            }
            if next > cap {
                next = cap;
            }
        }
    }
    let stored = u32::try_from(next).map_err(|_| crate::error::Reason::OutOfRange)?;
    let changed = stored != u32::try_from(current).unwrap_or(0);
    if stored == 0 {
        counts.remove(id);
    } else {
        counts.insert(id.to_string(), stored);
    }
    Ok(changed)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// How many times an actor has acted, and the journey they are still on.
pub struct ActCounters {
    pub last_act_tick: u64,
    pub idle_acts_today: u32,
    pub live_acts_this_hour: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub busy_until: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub busy_for: Option<PendingAction>,
}

impl ActCounters {
    pub fn fresh() -> Self {
        Self {
            last_act_tick: 0,
            idle_acts_today: 0,
            live_acts_this_hour: 0,
            busy_until: None,
            busy_for: None,
        }
    }
}

/// Work that finishes when `busy_until` is reached. Travel keeps the destination.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingAction {
    pub rule: String,
    pub verb: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arrive: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
    pub source: crate::intent::Source,
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
pub struct Message {
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
    pub tick_index: u64,
    pub actor_id: String,
    pub intent_id: String,
    pub fact_ids: Vec<String>,
    pub tags: Vec<String>,
    pub text: Option<String>,
}
