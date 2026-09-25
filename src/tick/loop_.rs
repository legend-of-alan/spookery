use std::collections::BTreeSet;
use std::path::Path;

use spookery_kernel::apply::apply_effect;
use spookery_kernel::blake2s_256;
use spookery_kernel::check::resolve_check;
use spookery_kernel::derive::{
    filter_blocklist, kernel_intents_for_eligible_npcs, legal, season_intent,
};
use spookery_kernel::expand::expand;
use spookery_kernel::model::{ActorKind, Intent};
use spookery_kernel::moves::needs_roll;
use spookery_kernel::pause::apply_pause_policy;
use spookery_kernel::perceive::perceive_at;
use spookery_kernel::store::{
    lock_shard, open_shard, session_status, tick_commit, PendingCommit, Shard, Txn,
};
use spookery_kernel::Code;

use crate::advance::{pick_advance, plan_catchup, Catchup};
use crate::budget::prioritize;
use crate::log::{render_tick, Rejected};
use crate::seed::seed_preimage;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    Scheduler,
    Catchup,
    Flush,
    Manual,
    Replay,
}

pub struct RecordedTick {
    pub intents: Vec<Intent>,
    pub presence_humans: u32,
}

pub fn commit_turn(shard: &mut Shard) -> Result<spookery_kernel::store::CommitOutcome, Code> {
    let _lock = lock_shard(shard)?;
    let wall = shard.now().last_tick_at.clone();
    run_tick(shard, Reason::Flush, &wall, Catchup::none())?;
    spookery_kernel::store::commit_pending(shard)
}

pub fn tick_pack(path: &Path) -> Result<(), Code> {
    let mut shard = open_shard(path)?;
    let wall = shard.now().last_tick_at.clone();
    run_tick(&mut shard, Reason::Manual, &wall, Catchup::none())
}

pub fn run_catchup(shard: &mut Shard, now_wall: &str, downtime_hours: u64) -> Result<(), Code> {
    let humans = session_status(shard.now()).humans;
    let plan = plan_catchup(humans, shard.budget(), downtime_hours)?;
    let ticks = plan.ticks_to_run;
    for index in 1..=ticks {
        let last = index == ticks;
        run_tick(
            shard,
            Reason::Catchup,
            now_wall,
            Catchup {
                downtime_hours: Some(downtime_hours),
                skipped: if last { plan.skipped } else { 0 },
                last,
            },
        )?;
    }
    Ok(())
}

pub fn run_tick(
    shard: &mut Shard,
    reason: Reason,
    now_wall: &str,
    catchup: Catchup,
) -> Result<(), Code> {
    run_tick_with(shard, reason, now_wall, catchup, None)
}

pub(crate) fn run_recorded_tick(
    shard: &mut Shard,
    now_wall: &str,
    recorded: RecordedTick,
) -> Result<(), Code> {
    run_tick_with(
        shard,
        Reason::Replay,
        now_wall,
        Catchup::none(),
        Some(recorded),
    )
}

fn run_tick_with(
    shard: &mut Shard,
    reason: Reason,
    now_wall: &str,
    catchup: Catchup,
    recorded: Option<RecordedTick>,
) -> Result<(), Code> {
    let _lock = if reason == Reason::Flush {
        None
    } else {
        Some(lock_shard(shard)?)
    };
    let mut txn = Txn::begin(shard);
    let queued = shard.queued_intents().to_vec();
    let owns_commit = reason != Reason::Flush;
    let record = stage_tick(
        shard,
        &mut txn,
        queued,
        now_wall,
        &catchup,
        owns_commit,
        recorded.as_ref(),
    )?;
    if reason == Reason::Flush {
        shard.set_pending(PendingCommit {
            now: txn.now().clone(),
            tick_jsonl: record,
            tick_index: txn.now().tick_index,
            proposals: txn.proposals().to_vec(),
            memories: txn.memories().to_vec(),
            paths: txn.staged_paths(),
        });
        return Ok(());
    }
    tick_commit(shard, txn, &record)
}

fn stage_tick(
    shard: &Shard,
    txn: &mut Txn,
    queued: Vec<Intent>,
    now_wall: &str,
    catchup: &Catchup,
    owns_commit: bool,
    recorded: Option<&RecordedTick>,
) -> Result<String, Code> {
    let rev_before = txn.now().state_rev;
    let world_before = txn.now().world_time;
    let humans = recorded
        .map(|recorded| recorded.presence_humans)
        .unwrap_or_else(|| session_status(txn.now()).humans);
    let tick_index = txn.now().tick_index + 1;
    let shard_id = txn.now().shard_id.clone();
    txn.pin_loop(tick_index, world_before, now_wall);
    let minutes = pick_advance(humans, catchup, txn.budget());
    let world_after = world_before.saturating_add(minutes);
    let day_changed = world_before / 1440 != world_after / 1440;
    let hour_changed = world_before / 60 != world_after / 60;
    txn.pin_loop(tick_index, world_after, now_wall);
    if day_changed {
        txn.reset_idle_acts()?;
    }
    if hour_changed {
        txn.reset_live_acts()?;
    }
    apply_pause_policy(txn)?;

    let mut trace = vec!["session_status".to_string()];
    let mut rejected = Vec::new();
    let candidates = if let Some(recorded) = recorded {
        recorded.intents.clone()
    } else {
        let mut candidates = queued;
        candidates.extend(kernel_intents_for_eligible_npcs(txn.canon(), txn.now()));
        let (candidates, blocked) = filter_blocklist(candidates, humans);
        for (intent, code) in blocked {
            rejected.push(Rejected {
                id: intent.id,
                reason: code.jsonl_reason().to_string(),
            });
        }
        let mut candidates = prioritize(txn.canon(), txn.now(), candidates, tick_index, &shard_id);
        let max_intents = txn.budget().max_intents_applied as usize;
        if candidates.len() > max_intents {
            candidates.truncate(max_intents);
        }
        candidates
    };
    let mut seen = BTreeSet::new();
    for intent in &candidates {
        if !seen.insert(intent.id.clone()) {
            return Err(Code::Schema);
        }
    }

    let max_npcs = txn.budget().max_npcs_acting;
    let max_effects = txn.budget().max_effects_per_intent as usize;
    let mut npc_slots = 0u64;
    let mut applied = Vec::new();
    let mut effects_out = Vec::new();
    let mut budget_stop = false;
    for intent in candidates {
        if budget_stop {
            break;
        }
        let spends_slot = intent.actor_kind == ActorKind::Npc && intent.move_name != "wait";
        if spends_slot && npc_slots >= max_npcs {
            rejected.push(Rejected {
                id: intent.id,
                reason: Code::Budget.jsonl_reason().to_string(),
            });
            continue;
        }
        trace.push(format!("perceive:{}", intent.actor_id));
        let view = perceive_at(
            txn.canon(),
            txn.now(),
            &viewer_token(&intent)?,
        )?;
        if view.known_facts.is_empty() {
            rejected.push(Rejected {
                id: intent.id,
                reason: "empty_perceive".to_string(),
            });
            continue;
        }
        if let Err(code) = legal(txn.canon(), txn.now(), &intent) {
            rejected.push(Rejected {
                id: intent.id,
                reason: code.jsonl_reason().to_string(),
            });
            continue;
        }
        if needs_roll(txn.canon(), &intent.move_name) {
            trace.push("resolve_check".to_string());
            let check_id = txn
                .canon()
                .moves
                .get(&intent.move_name)
                .and_then(|check| check.clone())
                .ok_or(Code::UnknownKey)?;
            let viewer = spookery_kernel::model::ViewerId::from_kind(intent.actor_kind, &intent.actor_id)?;
            let seed = seed_preimage(&shard_id, tick_index);
            let rng = blake2s_256(seed.as_bytes());
            let mut cursor = 0usize;
            if let Err(code) = resolve_check(
                txn.canon(),
                shard.sheets(),
                &viewer,
                &check_id,
                0,
                &rng,
                &mut cursor,
            ) {
                rejected.push(Rejected {
                    id: intent.id,
                    reason: code.jsonl_reason().to_string(),
                });
                continue;
            }
        }
        let expanded = match expand(txn.canon(), txn.now(), &intent) {
            Ok(effects) => effects,
            Err(code) => {
                rejected.push(Rejected {
                    id: intent.id,
                    reason: code.jsonl_reason().to_string(),
                });
                continue;
            }
        };
        let expanded_empty = expanded.is_empty();
        let mut accepted = 0usize;
        let mut counted = 0usize;
        let mut failed = false;
        for effect in expanded {
            if counted >= max_effects {
                break;
            }
            match apply_effect(txn, &intent, effect.clone()) {
                Ok(()) => {
                    accepted += 1;
                    counted += 1;
                    trace.push("apply_effect".to_string());
                    effects_out.push(effect);
                    crash_after_apply_effect();
                }
                Err(Code::Skipped) => rejected.push(Rejected {
                    id: intent.id.clone(),
                    reason: Code::Skipped.jsonl_reason().to_string(),
                }),
                Err(Code::Budget) => {
                    budget_stop = true;
                    if accepted == 0 {
                        rejected.push(Rejected {
                            id: intent.id.clone(),
                            reason: Code::Budget.jsonl_reason().to_string(),
                        });
                    }
                    break;
                }
                Err(code) => {
                    rejected.push(Rejected {
                        id: intent.id.clone(),
                        reason: code.jsonl_reason().to_string(),
                    });
                    failed = true;
                    break;
                }
            }
        }
        if budget_stop && accepted > 0 {
            applied.push(intent);
            break;
        }
        if budget_stop {
            break;
        }
        if failed && accepted == 0 {
            continue;
        }
        if accepted > 0 || (expanded_empty && !failed) {
            if intent.actor_kind == ActorKind::Npc {
                match txn.stage_npc_counters(&intent.actor_id, humans) {
                    Ok(()) => {}
                    Err(Code::Budget) => {
                        if accepted == 0 {
                            rejected.push(Rejected {
                                id: intent.id,
                                reason: Code::Budget.jsonl_reason().to_string(),
                            });
                        } else {
                            applied.push(intent);
                        }
                        break;
                    }
                    Err(code) => {
                        rejected.push(Rejected {
                            id: intent.id,
                            reason: code.jsonl_reason().to_string(),
                        });
                        continue;
                    }
                }
            }
            if spends_slot {
                npc_slots += 1;
            }
            applied.push(intent);
        }
    }

    if catchup.skipped > 0 && catchup.last {
        if let Some(intent) = season_intent(txn.now()) {
            if seen.insert(intent.id.clone()) {
                match expand(txn.canon(), txn.now(), &intent) {
                    Ok(effects) => {
                        let mut accepted = 0;
                        for effect in effects {
                            match apply_effect(txn, &intent, effect.clone()) {
                                Ok(()) => {
                                    accepted += 1;
                                    trace.push("apply_effect".to_string());
                                    effects_out.push(effect);
                                }
                                Err(code) => {
                                    rejected.push(Rejected {
                                        id: intent.id.clone(),
                                        reason: code.jsonl_reason().to_string(),
                                    });
                                    break;
                                }
                            }
                        }
                        if accepted > 0 {
                            applied.push(intent);
                        }
                    }
                    Err(code) => rejected.push(Rejected {
                        id: intent.id,
                        reason: code.jsonl_reason().to_string(),
                    }),
                }
            }
        }
    }

    if owns_commit {
        trace.push("tick_commit".to_string());
    }
    Ok(render_tick(
        tick_index,
        rev_before,
        rev_before + 1,
        world_before,
        world_after,
        humans,
        &seed_preimage(&shard_id, tick_index),
        now_wall,
        &applied,
        &rejected,
        &effects_out,
        &trace,
    ))
}

fn crash_after_apply_effect() {
    if std::env::var_os("SPOOK_CRASH_AFTER").is_some_and(|value| value == "apply_effect") {
        std::process::abort();
    }
}

fn viewer_token(intent: &Intent) -> Result<String, Code> {
    match intent.actor_kind {
        ActorKind::Npc => Ok(format!("npc:{}", intent.actor_id)),
        ActorKind::Player => Ok(format!("player:{}", intent.actor_id)),
        ActorKind::System => Err(Code::UnscopedPerceive),
    }
}

