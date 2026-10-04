use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::Reason;

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
    pub proposals: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Presence {
    pub humans: u32,
    pub session_ids: Vec<String>,
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
    pub travel: RunState,
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
        needs: BTreeMap<String, i64>,
        resources: BTreeMap<String, i64>,
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
        known_facts: Vec<String>,
        sheet: String,
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

    pub fn needs(&self) -> Option<&BTreeMap<String, i64>> {
        match self {
            ActorLive::Npc { needs, .. } => Some(needs),
            ActorLive::Pc { .. } => None,
        }
    }

    pub fn needs_mut(&mut self) -> Option<&mut BTreeMap<String, i64>> {
        match self {
            ActorLive::Npc { needs, .. } => Some(needs),
            ActorLive::Pc { .. } => None,
        }
    }

    pub fn resources_mut(&mut self) -> Option<&mut BTreeMap<String, i64>> {
        match self {
            ActorLive::Npc { resources, .. } => Some(resources),
            ActorLive::Pc { .. } => None,
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PcSheet {
    pub schema: String,
    pub id: String,
    pub template: String,
    pub stats: BTreeMap<String, i64>,
    pub resources: BTreeMap<String, i64>,
    pub inventory: Vec<String>,
    pub flags: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposalDoc {
    pub schema: String,
    pub id: String,
    pub kind: String,
    pub target: String,
    pub summary: String,
    pub actor_id: String,
    pub source: String,
    pub tick_index: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ViewerId {
    Player(String),
    Npc(String),
}

impl ViewerId {
    pub fn parse(raw: &str) -> Result<Self, Reason> {
        let Some((kind, id)) = raw.split_once(':') else {
            return Err(Reason::UnscopedPerceive);
        };
        if id.is_empty() {
            return Err(Reason::UnscopedPerceive);
        }
        match kind {
            "player" => Ok(ViewerId::Player(id.to_string())),
            "npc" => Ok(ViewerId::Npc(id.to_string())),
            _ => Err(Reason::UnscopedPerceive),
        }
    }

    pub fn actor_id(&self) -> &str {
        match self {
            ViewerId::Player(id) | ViewerId::Npc(id) => id,
        }
    }

    pub fn token(&self) -> String {
        match self {
            ViewerId::Player(id) => format!("player:{id}"),
            ViewerId::Npc(id) => format!("npc:{id}"),
        }
    }

    pub fn from_kind(kind: ActorKind, id: &str) -> Result<Self, Reason> {
        match kind {
            ActorKind::Npc => Ok(ViewerId::Npc(id.to_string())),
            ActorKind::Player => Ok(ViewerId::Player(id.to_string())),
            ActorKind::System => Err(Reason::UnscopedPerceive),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActorKind {
    Npc,
    Player,
    System,
}

impl ActorKind {
    pub fn token(self) -> &'static str {
        match self {
            ActorKind::Npc => "npc",
            ActorKind::Player => "player",
            ActorKind::System => "system",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Kernel,
    Llm,
    Player,
    System,
}

impl Source {
    pub fn token(self) -> &'static str {
        match self {
            Source::Kernel => "kernel",
            Source::Llm => "llm",
            Source::Player => "player",
            Source::System => "system",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Target {
    #[default]
    Empty,
    Place(String),
    Actor(String),
    Clock(String),
    Rumor(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct IntentArgs {
    pub sku: Option<String>,
    pub delta: Option<i64>,
    pub key: Option<String>,
    pub to: Option<String>,
    pub message_id: Option<String>,
    pub due_clock: Option<String>,
    pub due_value: Option<i64>,
    pub place: Option<String>,
    pub fact_ids: Option<Vec<String>>,
    pub message_to: Option<Vec<String>>,
    pub text: Option<String>,
    pub proposal_id: Option<String>,
    pub kind: Option<String>,
    pub target: Option<String>,
    /// Integer target for `attempt`. Serialized as args `target` when set.
    pub check_target: Option<i64>,
    pub check: Option<String>,
    pub summary: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Intent {
    pub id: String,
    pub tick_index: u64,
    pub actor_id: String,
    pub actor_kind: ActorKind,
    pub source: Source,
    pub move_name: String,
    pub target: Target,
    pub args: IntentArgs,
    pub rationale_code: String,
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
