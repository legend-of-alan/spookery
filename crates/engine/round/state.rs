//! One pass of the rules over one tick.

use std::collections::BTreeMap;

use crate::draft::Draft;
use crate::effect::Effect;
use crate::hash::blake2s_256;
use crate::intent::Intent;
use crate::journal::{Arrival, Rejected};
use crate::key::ChangeRecord;
use crate::model::Now;
use crate::round::play::{RecordedRound, RoundCause};
use crate::rules::gate::GateRefusal;
use crate::rules::{MoveReport, RollResult};
use crate::visibility::{RecipientLine, ViewerChanges};
use crate::world::World;

/// The round line's `seed` field. A roll uses `RollKey`, not this string.
pub(crate) fn round_seed(world_id: &str, round_index: u64) -> String {
    format!("{world_id}|{round_index}")
}

pub(crate) fn shuffle_key(world_id: &str, round_index: u64, actor_id: &str) -> [u8; 32] {
    blake2s_256(format!("{world_id}|{round_index}|{actor_id}").as_bytes())
}

/// One pass of the rules over one tick.
pub(crate) struct Round {
    pub round_index: u64,
    pub world_time_before: u64,
    pub world_time_after: u64,
    pub humans: u32,
    pub world_id: String,
    pub now_wall: String,
    pub rev_before: u64,
    pub queued: Vec<Intent>,
    pub recorded: Option<RecordedRound>,
    pub candidates: Vec<Intent>,
    pub applied: Vec<Intent>,
    pub rejected: Vec<Rejected>,
    pub effects: Vec<Effect>,
    pub arrivals: Vec<Arrival>,
    pub fast_forward: bool,
    pub steps_run: Vec<String>,
    pub move_reports: Vec<MoveReport>,
    pub changes: Vec<ChangeRecord>,
    pub summary: Vec<String>,
    pub lines: Vec<RecipientLine>,
    pub seen: Vec<ViewerChanges>,
    pub opposed: BTreeMap<String, RollResult>,
    pub roll_counts: BTreeMap<(String, String), u32>,
    pub dice: Vec<String>,
    pub resolutions: BTreeMap<String, Resolution>,
    pub frozen: Now,
    pub budget_stop: bool,
    pub opposing_candidates: Vec<Intent>,
}

/// How resolve left an intent for apply.
#[derive(Clone)]
pub(crate) enum Resolution {
    Ready {
        rolled: Option<RollResult>,
    },
    Refused {
        refusal: GateRefusal,
        report: Option<MoveReport>,
    },
}

impl Round {
    pub(crate) fn prepare(
        world: &World,
        draft: &Draft,
        cause: RoundCause,
        now_wall: &str,
        recorded: Option<RecordedRound>,
    ) -> Self {
        let humans = recorded
            .as_ref()
            .map(|recorded| recorded.presence_humans)
            .unwrap_or(draft.now().presence.humans);
        Self {
            round_index: draft.now().round_index,
            world_time_before: draft.now().world_time,
            world_time_after: draft.now().world_time,
            humans,
            world_id: draft.now().shard_id.clone(),
            now_wall: now_wall.to_string(),
            rev_before: draft.now().state_rev,
            queued: world.queued_intents().to_vec(),
            recorded,
            candidates: Vec::new(),
            applied: Vec::new(),
            rejected: Vec::new(),
            effects: Vec::new(),
            arrivals: Vec::new(),
            fast_forward: cause == RoundCause::FastForward,
            steps_run: Vec::new(),
            move_reports: Vec::new(),
            changes: Vec::new(),
            summary: Vec::new(),
            lines: Vec::new(),
            seen: Vec::new(),
            opposed: BTreeMap::new(),
            roll_counts: BTreeMap::new(),
            dice: Vec::new(),
            resolutions: BTreeMap::new(),
            frozen: draft.now().clone(),
            budget_stop: false,
            opposing_candidates: Vec::new(),
        }
    }
}
