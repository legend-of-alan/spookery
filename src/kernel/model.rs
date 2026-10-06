use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::meter::MeterLive;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Now {
    pub schema: String,
    pub shard_id: String,
    pub pack_rev: String,
    pub state_rev: u64,
    pub tick_index: u64,
    pub world_time: u64,
    pub last_tick_at: String,
    pub presence: Presence,
    pub pause: PauseView,
    pub places: BTreeMap<String, PlaceLive>,
    pub actors: BTreeMap<String, ActorLive>,
    pub clocks: BTreeMap<String, Clock>,
    pub rumors: Vec<Rumor>,
    pub messages: Vec<Message>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
// Emit order is the docks `now.yaml` pause mapping.
pub struct PauseView {
    pub named_plot_clocks: RunState,
    pub relationship_drift: RunState,
    pub subsistence_clocks: RunState,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaceLive {
    pub stock: BTreeMap<String, i64>,
    pub hours: BTreeMap<String, i64>,
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
        busy: bool,
        meters: BTreeMap<String, MeterLive>,
        inventory: Vec<String>,
        relationships: BTreeMap<String, Relation>,
        reputation: BTreeMap<String, Reputation>,
        known_facts: Vec<String>,
        tick: TickCounters,
    },
    #[serde(rename = "pc")]
    Pc {
        place: String,
        flags: Vec<String>,
        busy: bool,
        meters: BTreeMap<String, MeterLive>,
        inventory: Vec<String>,
        known_facts: Vec<String>,
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

    pub fn busy(&self) -> bool {
        match self {
            ActorLive::Npc { busy, .. } | ActorLive::Pc { busy, .. } => *busy,
        }
    }

    pub fn busy_mut(&mut self) -> &mut bool {
        match self {
            ActorLive::Npc { busy, .. } | ActorLive::Pc { busy, .. } => busy,
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

    pub fn tick(&self) -> Option<&TickCounters> {
        match self {
            ActorLive::Npc { tick, .. } => Some(tick),
            ActorLive::Pc { .. } => None,
        }
    }

    pub fn tick_mut(&mut self) -> Option<&mut TickCounters> {
        match self {
            ActorLive::Npc { tick, .. } => Some(tick),
            ActorLive::Pc { .. } => None,
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

    pub fn inventory(&self) -> &[String] {
        match self {
            ActorLive::Npc { inventory, .. } | ActorLive::Pc { inventory, .. } => inventory,
        }
    }

    pub fn relationships(&self) -> Option<&BTreeMap<String, Relation>> {
        match self {
            ActorLive::Npc { relationships, .. } => Some(relationships),
            ActorLive::Pc { .. } => None,
        }
    }

    pub fn relationships_mut(&mut self) -> Option<&mut BTreeMap<String, Relation>> {
        match self {
            ActorLive::Npc { relationships, .. } => Some(relationships),
            ActorLive::Pc { .. } => None,
        }
    }

    pub fn reputation_mut(&mut self) -> Option<&mut BTreeMap<String, Reputation>> {
        match self {
            ActorLive::Npc { reputation, .. } => Some(reputation),
            ActorLive::Pc { .. } => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Relation {
    pub stance: i64,
    pub target: i64,
    pub last_interaction_world_time: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reputation {
    pub value: i64,
    pub last_world_time: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TickCounters {
    pub last_act_tick: u64,
    pub idle_acts_today: u32,
    pub live_acts_this_hour: u32,
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
pub struct Rumor {
    pub id: String,
    pub fact_ids: Vec<String>,
    pub origin: String,
    pub heat: i64,
    pub known_by: Vec<String>,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryLine {
    pub tick_index: u64,
    pub actor_id: String,
    pub intent_id: String,
    pub fact_ids: Vec<String>,
    pub tags: Vec<String>,
    pub text: Option<String>,
}
