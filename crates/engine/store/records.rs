//! Journal record shapes. Sealing and the bundle files live in `journal`.

use serde::{Deserialize, Serialize};

use crate::effect::Effect;
use crate::error::Reason;
use crate::intent::Intent;
use crate::model::MemoryLine;
use crate::rules::GateRefusal;

/// One meter a migrate added, stored at that meter's `start`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AddedMeter {
    pub id: String,
    pub start: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MigrateRecord {
    pub pack_rev_before: String,
    pub pack_rev_after: String,
    pub state_rev_before: u64,
    pub state_rev_after: u64,
    pub state_hash: String,
    /// Meters this migrate added. Omitted when it added none, so older lines stay the same.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub meters: Vec<AddedMeter>,
    /// The migrate started from a hand-edited save, and a checkpoint of its result follows it.
    /// Omitted when false, so a plain migrate keeps its bytes.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub edited: bool,
    /// Chain hash of the previous record. Empty on a record that has not been sealed.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub prev: String,
    /// Blake2s of `prev` and this record's bytes, not including `chain` itself.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub chain: String,
}

/// Written at a checkpoint. `state_hash` is the Blake2s of `save.yaml` after that round.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckpointRecord {
    pub round_index: u64,
    pub state_rev: u64,
    pub state_hash: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub prev: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub chain: String,
}

/// One refusal stored on a round line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Refusal {
    pub id: String,
    pub code: Reason,
    /// The cost shortfall, or the sentence a locked exit shows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl Refusal {
    pub(crate) fn new(intent: &Intent, refusal: &GateRefusal) -> Self {
        Self {
            id: intent.id.clone(),
            code: refusal.reason(),
            detail: refusal.missing().map(str::to_string),
        }
    }
}

/// One step off the standard order of a round.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    CheckRolled,
    EffectApplied,
    Rule(String),
    TriggerDepth,
}

/// One timed arrival stored on a round line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Arrival {
    pub actor_id: String,
    pub arrived_at: u64,
    pub to: String,
    pub via: String,
}

/// One round line. `state_hash` is stamped after the save bytes exist, so a line
/// that has not been stamped omits it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoundRecord {
    pub round_index: u64,
    #[serde(default)]
    pub rules_rev: u64,
    pub state_rev_before: u64,
    pub state_rev_after: u64,
    pub world_time_before: u64,
    pub world_time_after: u64,
    pub presence_players: u32,
    /// Seated actor ids when the round ran.
    pub actors_present: Vec<String>,
    pub seed: String,
    pub last_round_at: String,
    /// The intents queued for this round, before stamping and habits, which replay plays again.
    pub intents: Vec<Intent>,
    pub refused: Vec<Refusal>,
    pub effects: Vec<Effect>,
    pub arrivals: Vec<Arrival>,
    pub fast_forward: bool,
    /// Dice lines from this round. Empty when the round rolled none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dice: Vec<String>,
    /// Steps off the standard order. Empty when the round followed it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_steps: Vec<Step>,
    /// Lines for this round. Empty on a record that stored none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub memories: Vec<MemoryLine>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub state_hash: String,
    /// Chain hash of the previous record. Empty on a record that has not been sealed.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub prev: String,
    /// Blake2s of `prev` and this record's bytes, not including `chain` itself.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub chain: String,
}

/// One journal line. `kind` is its first key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum Record {
    Round(RoundRecord),
    Migrate(MigrateRecord),
    Checkpoint(CheckpointRecord),
}

impl Record {
    pub fn state_rev_after(&self) -> u64 {
        match self {
            Self::Round(record) => record.state_rev_after,
            Self::Migrate(record) => record.state_rev_after,
            Self::Checkpoint(record) => record.state_rev,
        }
    }

    pub fn state_rev_before(&self) -> u64 {
        match self {
            Self::Round(record) => record.state_rev_before,
            Self::Migrate(record) => record.state_rev_before,
            Self::Checkpoint(record) => record.state_rev,
        }
    }

    #[cfg(any(test, feature = "testing"))]
    pub fn state_hash(&self) -> &str {
        match self {
            Self::Round(record) => &record.state_hash,
            Self::Migrate(record) => &record.state_hash,
            Self::Checkpoint(record) => &record.state_hash,
        }
    }

    pub fn prev(&self) -> &str {
        match self {
            Self::Round(record) => &record.prev,
            Self::Migrate(record) => &record.prev,
            Self::Checkpoint(record) => &record.prev,
        }
    }

    pub fn chain(&self) -> &str {
        match self {
            Self::Round(record) => &record.chain,
            Self::Migrate(record) => &record.chain,
            Self::Checkpoint(record) => &record.chain,
        }
    }

    pub(crate) fn set_prev(&mut self, prev: String) {
        match self {
            Self::Round(record) => record.prev = prev,
            Self::Migrate(record) => record.prev = prev,
            Self::Checkpoint(record) => record.prev = prev,
        }
    }

    pub(crate) fn set_chain(&mut self, chain: String) {
        match self {
            Self::Round(record) => record.chain = chain,
            Self::Migrate(record) => record.chain = chain,
            Self::Checkpoint(record) => record.chain = chain,
        }
    }

    pub fn round_index(&self) -> Option<u64> {
        match self {
            Self::Round(record) => Some(record.round_index),
            Self::Checkpoint(record) => Some(record.round_index),
            Self::Migrate(_) => None,
        }
    }

    /// A round or migrate moves `state_rev`. A checkpoint marks the state just written.
    pub fn changes_state(&self) -> bool {
        !matches!(self, Self::Checkpoint(_))
    }
}
