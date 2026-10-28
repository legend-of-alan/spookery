//! Journal record shapes. Sealing and the bundle files live in `journal`.

use serde::{Deserialize, Serialize};

use crate::effect::Effect;
use crate::error::Reason;
use crate::intent::Intent;
use crate::model::MemoryLine;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MigrateKind {
    Migrate,
}

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
    pub(crate) kind: MigrateKind,
    pub pack_rev_before: String,
    pub pack_rev_after: String,
    pub state_rev_before: u64,
    pub state_rev_after: u64,
    pub now_hash: String,
    /// Meters this migrate added. Omitted when it added none, so older lines stay the same.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub meters: Vec<AddedMeter>,
    /// Chain hash of the previous record. Empty on a record that has not been sealed.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub prev: String,
    /// Blake2s of `prev` and this record's bytes, not including `chain` itself.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub chain: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CheckpointKind {
    Checkpoint,
}

/// Written at a checkpoint. `state_hash` is the Blake2s of `save.yaml` after that round.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckpointRecord {
    pub(crate) kind: CheckpointKind,
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
pub struct Rejected {
    pub id: String,
    pub code: Reason,
    /// The cost shortfall, or the sentence a locked exit shows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missing: Option<String>,
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

/// One round line. `now_hash` is stamped after the save bytes exist, so a line
/// that has not been stamped omits it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoundRecord<E = Effect> {
    pub round_index: u64,
    #[serde(default)]
    pub rules_rev: u64,
    pub state_rev_before: u64,
    pub state_rev_after: u64,
    pub world_time_before: u64,
    pub world_time_after: u64,
    pub presence_humans: u32,
    /// Seated actor ids when the round ran.
    pub actors_present: Vec<String>,
    pub seed: String,
    pub last_tick_at: String,
    /// The intents queued for this round, before stamping and habits, which replay plays again.
    pub intents: Vec<Intent>,
    pub rejected: Vec<Rejected>,
    pub effects: Vec<E>,
    pub arrivals: Vec<Arrival>,
    pub fast_forward: bool,
    /// Dice lines from this round. Empty when the round rolled none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dice: Vec<String>,
    /// Steps off the standard order. Empty when the round followed it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps_run: Vec<Step>,
    /// Lines for this round. Empty on a record that stored none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub memories: Vec<MemoryLine>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub now_hash: String,
    /// Chain hash of the previous record. Empty on a record that has not been sealed.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub prev: String,
    /// Blake2s of `prev` and this record's bytes, not including `chain` itself.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub chain: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)]
pub enum JournalEntry {
    Round(RoundRecord),
    Migrate(MigrateRecord),
    Checkpoint(CheckpointRecord),
}

impl JournalEntry {
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

    #[allow(dead_code)]
    pub fn now_hash(&self) -> &str {
        match self {
            Self::Round(record) => &record.now_hash,
            Self::Migrate(record) => &record.now_hash,
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
