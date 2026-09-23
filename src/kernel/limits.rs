use serde::Deserialize;

use crate::error::Code;

pub const MINUTES_PER_HOUR: u64 = 60;
pub const MINUTES_PER_DAY: u64 = 1440;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget {
    pub max_intents_applied: u64,
    pub max_npcs_acting: u64,
    pub llm_npc_intents_per_tick: u64,
    pub max_effects_per_intent: u64,
    pub max_state_keys_mutated: u64,
    pub max_memory_writes: u64,
    pub max_rumor_hops: u64,
    pub idle_acts_per_world_day: u64,
    pub live_acts_per_world_hour: u64,
    pub tick_every_real_seconds: u64,
    pub live_tick_every_real_seconds: u64,
    pub idle_minutes_per_tick: u64,
    pub live_minutes_per_tick: u64,
    pub idle_catchup_max_ticks: u64,
    pub idle_max_world_hours_per_real_hour: u64,
    pub pause: PausePolicies,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PausePolicies {
    pub named_plot_clocks: PausePolicy,
    pub subsistence_clocks: PausePolicy,
    pub relationship_drift: PausePolicy,
    pub travel: PausePolicy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub enum PausePolicy {
    #[serde(rename = "pause_when_empty")]
    PauseWhenEmpty,
    #[serde(rename = "always_run")]
    AlwaysRun,
    #[serde(rename = "run_if_schedule")]
    RunIfSchedule,
}

impl Budget {
    pub fn defaults() -> Self {
        Self {
            max_intents_applied: 12,
            max_npcs_acting: 8,
            llm_npc_intents_per_tick: 0,
            max_effects_per_intent: 4,
            max_state_keys_mutated: 24,
            max_memory_writes: 8,
            max_rumor_hops: 2,
            idle_acts_per_world_day: 3,
            live_acts_per_world_hour: 6,
            tick_every_real_seconds: 300,
            live_tick_every_real_seconds: 60,
            idle_minutes_per_tick: 60,
            live_minutes_per_tick: 5,
            idle_catchup_max_ticks: 12,
            idle_max_world_hours_per_real_hour: 8,
            pause: PausePolicies {
                named_plot_clocks: PausePolicy::PauseWhenEmpty,
                subsistence_clocks: PausePolicy::AlwaysRun,
                relationship_drift: PausePolicy::AlwaysRun,
                travel: PausePolicy::RunIfSchedule,
            },
        }
    }
}

pub fn world_hour(world_time: u64) -> i64 {
    ((world_time / MINUTES_PER_HOUR) % 24) as i64
}

pub fn load_tick_rules(text: &str) -> Result<Budget, Code> {
    let file: TickFile = serde_saphyr::from_str(text).map_err(|_| Code::Schema)?;
    if file.schema != "tick/v0" {
        return Err(Code::Schema);
    }
    let defaults = Budget::defaults();
    Ok(Budget {
        max_intents_applied: take(file.max_intents_applied, defaults.max_intents_applied)?,
        max_npcs_acting: take(file.max_npcs_acting, defaults.max_npcs_acting)?,
        llm_npc_intents_per_tick: take(
            file.llm_npc_intents_per_tick,
            defaults.llm_npc_intents_per_tick,
        )?,
        max_effects_per_intent: take(file.max_effects_per_intent, defaults.max_effects_per_intent)?,
        max_state_keys_mutated: take(
            file.max_state_keys_mutated,
            defaults.max_state_keys_mutated,
        )?,
        max_memory_writes: take(file.max_memory_writes, defaults.max_memory_writes)?,
        max_rumor_hops: take(file.max_rumor_hops, defaults.max_rumor_hops)?,
        idle_acts_per_world_day: take(
            file.idle_acts_per_world_day,
            defaults.idle_acts_per_world_day,
        )?,
        live_acts_per_world_hour: take(
            file.live_acts_per_world_hour,
            defaults.live_acts_per_world_hour,
        )?,
        tick_every_real_seconds: take(
            file.tick_every_real_seconds,
            defaults.tick_every_real_seconds,
        )?,
        live_tick_every_real_seconds: take(
            file.live_tick_every_real_seconds,
            defaults.live_tick_every_real_seconds,
        )?,
        idle_minutes_per_tick: take(file.idle_minutes_per_tick, defaults.idle_minutes_per_tick)?,
        live_minutes_per_tick: take(file.live_minutes_per_tick, defaults.live_minutes_per_tick)?,
        idle_catchup_max_ticks: take(
            file.idle_catchup_max_ticks,
            defaults.idle_catchup_max_ticks,
        )?,
        idle_max_world_hours_per_real_hour: take(
            file.idle_max_world_hours_per_real_hour,
            defaults.idle_max_world_hours_per_real_hour,
        )?,
        pause: PausePolicies {
            named_plot_clocks: file.pause.named_plot_clocks,
            subsistence_clocks: file.pause.subsistence_clocks,
            relationship_drift: file.pause.relationship_drift,
            travel: file.pause.travel,
        },
    })
}

fn take(value: Option<i64>, default: u64) -> Result<u64, Code> {
    match value {
        None => Ok(default),
        Some(n) if n >= 0 => Ok(n as u64),
        Some(_) => Err(Code::Schema),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TickFile {
    schema: String,
    max_intents_applied: Option<i64>,
    max_npcs_acting: Option<i64>,
    llm_npc_intents_per_tick: Option<i64>,
    max_effects_per_intent: Option<i64>,
    max_state_keys_mutated: Option<i64>,
    max_memory_writes: Option<i64>,
    max_rumor_hops: Option<i64>,
    idle_acts_per_world_day: Option<i64>,
    live_acts_per_world_hour: Option<i64>,
    tick_every_real_seconds: Option<i64>,
    live_tick_every_real_seconds: Option<i64>,
    idle_minutes_per_tick: Option<i64>,
    live_minutes_per_tick: Option<i64>,
    idle_catchup_max_ticks: Option<i64>,
    idle_max_world_hours_per_real_hour: Option<i64>,
    pause: PauseFile,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PauseFile {
    named_plot_clocks: PausePolicy,
    subsistence_clocks: PausePolicy,
    relationship_drift: PausePolicy,
    travel: PausePolicy,
}
