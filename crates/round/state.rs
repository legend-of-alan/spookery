use std::collections::BTreeMap;

use crate::play::{RecordedRound, RoundCause};
use spookery_kernel::api::draft::Draft;
use spookery_kernel::api::effect::Effect;
use spookery_kernel::api::intent::Intent;
use spookery_kernel::api::journal::{Arrival, Rejected};
use spookery_kernel::api::key::ChangeRecord;
use spookery_kernel::api::model::Now;
use spookery_kernel::api::rules::{RollReport, RollResult};
use spookery_kernel::api::visibility::{Seen, Told};
use spookery_kernel::api::world::{session_status, World};

/// One pass of the rules over one tick.
pub struct Round {
    pub tick_index: u64,
    pub world_time_before: u64,
    pub world_time_after: u64,
    pub humans: u32,
    pub cause: RoundCause,
    pub world_id: String,
    pub now_wall: String,
    pub rev_before: u64,
    pub owns_commit: bool,
    pub queued: Vec<Intent>,
    pub recorded: Option<RecordedRound>,
    pub candidates: Vec<Intent>,
    pub applied: Vec<Intent>,
    pub rejected: Vec<Rejected>,
    pub effects: Vec<Effect>,
    pub arrivals: Vec<Arrival>,
    pub fast_forward: bool,
    pub steps_run: Vec<String>,
    pub notices: Vec<RollReport>,
    pub roll_reports: Vec<RollReport>,
    pub changes: Vec<ChangeRecord>,
    pub summary: Vec<String>,
    pub lines: Vec<Told>,
    pub seen: Vec<Seen>,
    pub opposed: BTreeMap<String, RollResult>,
    pub roll_counts: BTreeMap<(String, String), u32>,
    pub dice: Vec<String>,
    pub resolutions: BTreeMap<String, Resolution>,
    pub frozen: Now,
    pub budget_stop: bool,
    pub supply_from: Vec<Intent>,
}

/// How resolve left an intent for apply.
#[derive(Clone)]
pub enum Resolution {
    Ready {
        rolled: Option<RollResult>,
    },
    Refused {
        reason: String,
        notice: Option<RollReport>,
    },
}

impl Round {
    pub fn prepare(
        world: &World,
        draft: &Draft,
        cause: RoundCause,
        now_wall: &str,
        recorded: Option<RecordedRound>,
    ) -> Self {
        let humans = recorded
            .as_ref()
            .map(|recorded| recorded.presence_humans)
            .unwrap_or_else(|| session_status(draft.now()).humans);
        Self {
            tick_index: draft.now().tick_index,
            world_time_before: draft.now().world_time,
            world_time_after: draft.now().world_time,
            humans,
            cause,
            world_id: draft.now().shard_id.clone(),
            now_wall: now_wall.to_string(),
            rev_before: draft.now().state_rev,
            owns_commit: cause != RoundCause::Flush,
            queued: world.queued_intents().to_vec(),
            recorded,
            candidates: Vec::new(),
            applied: Vec::new(),
            rejected: Vec::new(),
            effects: Vec::new(),
            arrivals: Vec::new(),
            fast_forward: cause == RoundCause::FastForward,
            steps_run: Vec::new(),
            notices: Vec::new(),
            roll_reports: Vec::new(),
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
            supply_from: Vec::new(),
        }
    }
}
