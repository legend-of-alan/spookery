use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use spookery_kernel::agenda::{allowed, kernel_intents_for_eligible_npcs};
use spookery_kernel::apply::apply_effect;
use spookery_kernel::blake2s_256;
use spookery_kernel::check::{resolve_check, CheckNotice, CheckOut};
use spookery_kernel::commit::tick_commit;
use spookery_kernel::expand::expand_outcome;
use spookery_kernel::intent::{ActorKind, Intent, Source, Target, ViewerId};
use spookery_kernel::lock::lock_shard;
use spookery_kernel::pause::apply_pause_policy;
use spookery_kernel::perceive::perceive_at;
use spookery_kernel::shard::{open_shard, session_status, PendingCommit, Shard, Txn};
use spookery_kernel::Reason as KernelReason;

use crate::advance::{pick_advance, plan_catchup};
use crate::budget::prioritize;
use crate::log::{render_tick, Rejected};
use crate::seed::seed_preimage;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cause {
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

pub fn commit_turn(
    shard: &mut Shard,
) -> Result<spookery_kernel::commit::CommitOutcome, KernelReason> {
    let wall = shard.now().last_tick_at.clone();
    commit_turn_at(shard, &wall)
}

/// `include_tick: true` with `last_tick_at` set to `wall`. The caller supplies the stamp.
pub fn commit_turn_at(
    shard: &mut Shard,
    wall: &str,
) -> Result<spookery_kernel::commit::CommitOutcome, KernelReason> {
    let _lock = lock_shard(shard)?;
    run_tick(shard, Cause::Flush, wall)?;
    spookery_kernel::commit::commit_pending(shard)
}

pub fn tick_pack(path: &Path) -> Result<(), KernelReason> {
    let mut shard = open_shard(path)?;
    let wall = shard.now().last_tick_at.clone();
    run_tick(&mut shard, Cause::Manual, &wall)
}

pub fn run_catchup(
    shard: &mut Shard,
    now_wall: &str,
    downtime_hours: u64,
) -> Result<(), KernelReason> {
    let humans = session_status(shard.now()).humans;
    let ticks = plan_catchup(humans, shard.budget(), downtime_hours)?;
    for _ in 0..ticks {
        run_tick(shard, Cause::Catchup, now_wall)?;
    }
    Ok(())
}

pub fn run_tick(shard: &mut Shard, cause: Cause, now_wall: &str) -> Result<(), KernelReason> {
    run_tick_with(shard, cause, now_wall, None)
}

pub(crate) fn run_recorded_tick(
    shard: &mut Shard,
    now_wall: &str,
    recorded: RecordedTick,
) -> Result<(), KernelReason> {
    run_tick_with(shard, Cause::Replay, now_wall, Some(recorded))
}

fn run_tick_with(
    shard: &mut Shard,
    cause: Cause,
    now_wall: &str,
    recorded: Option<RecordedTick>,
) -> Result<(), KernelReason> {
    let _lock = if cause == Cause::Flush {
        None
    } else {
        Some(lock_shard(shard)?)
    };
    let mut txn = Txn::begin(shard);
    let queued = shard.queued_intents().to_vec();
    let owns_commit = cause != Cause::Flush;
    shard.set_rolls(Vec::new());
    let (record, notices) = stage_tick(&mut txn, queued, now_wall, owns_commit, recorded.as_ref())?;
    if cause == Cause::Flush {
        shard.set_pending(PendingCommit {
            now: txn.now().clone(),
            tick_jsonl: record,
            tick_index: txn.now().tick_index,
            memories: txn.memories().to_vec(),
            paths: txn.staged_paths(),
        });
        shard.set_rolls(notices);
        return Ok(());
    }
    tick_commit(shard, txn, &record)?;
    shard.set_rolls(notices);
    if recorded.is_none() {
        shard.clear_queue();
    }
    Ok(())
}

fn stage_tick(
    txn: &mut Txn,
    queued: Vec<Intent>,
    now_wall: &str,
    owns_commit: bool,
    recorded: Option<&RecordedTick>,
) -> Result<(String, Vec<CheckNotice>), KernelReason> {
    let rev_before = txn.now().state_rev;
    let world_before = txn.now().world_time;
    let humans = recorded
        .map(|recorded| recorded.presence_humans)
        .unwrap_or_else(|| session_status(txn.now()).humans);
    let tick_index = txn.now().tick_index + 1;
    let shard_id = txn.now().shard_id.clone();
    txn.pin_loop(tick_index, world_before, now_wall);
    let minutes = pick_advance(humans, txn.budget());
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
    let supply_from = match recorded {
        Some(recorded) => recorded.intents.clone(),
        None => queued.clone(),
    };
    let candidates = if let Some(recorded) = recorded {
        recorded.intents.clone()
    } else {
        let mut candidates = queued;
        candidates.extend(kernel_intents_for_eligible_npcs(txn.canon(), txn.now()));
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
            return Err(KernelReason::Schema);
        }
    }

    let max_npcs = txn.budget().max_npcs_acting;
    let max_effects = txn.budget().max_effects_per_intent as usize;
    let mut npc_slots = 0u64;
    let mut applied = Vec::new();
    let mut effects_out = Vec::new();
    let mut budget_stop = false;
    let mut notices = Vec::new();
    let mut die_cursor = 0usize;
    let mut supplied = BTreeMap::new();
    let frozen = txn.now().clone();
    for intent in candidates {
        if budget_stop {
            break;
        }
        let spends_slot = intent.actor_kind == ActorKind::Npc;
        if spends_slot && npc_slots >= max_npcs {
            rejected.push(Rejected {
                id: intent.id,
                reason: KernelReason::Budget.jsonl_reason().to_string(),
            });
            continue;
        }
        trace.push(format!("perceive:{}", intent.actor_id));
        let view = perceive_at(txn.canon(), txn.now(), &viewer_token(&intent)?)?;
        if view.known_facts.is_empty() {
            rejected.push(Rejected {
                id: intent.id,
                reason: "empty_perceive".to_string(),
            });
            continue;
        }
        if let Err(code) = allowed(txn.canon(), txn.now(), &intent) {
            rejected.push(Rejected {
                id: intent.id,
                reason: code.jsonl_reason().to_string(),
            });
            continue;
        }
        let mut rolled: Option<CheckOut> = None;
        if let Some(check_id) = txn.canon().row_check(&intent.move_name) {
            let viewer = match ViewerId::from_kind(intent.actor_kind, &intent.actor_id) {
                Ok(viewer) => viewer,
                Err(code) => {
                    reject_check(&mut rejected, &mut notices, &intent, code);
                    continue;
                }
            };
            trace.push("resolve_check".to_string());
            let seed = seed_preimage(&shard_id, tick_index);
            let rng = blake2s_256(seed.as_bytes());
            let target = match supplied_target(
                txn.canon(),
                &frozen,
                &supply_from,
                &intent,
                &rng,
                &mut die_cursor,
                &mut supplied,
            ) {
                Ok(opposed) => {
                    opposed.unwrap_or_else(|| txn.canon().row_difficulty(&intent.move_name))
                }
                Err(code) => {
                    reject_check(&mut rejected, &mut notices, &intent, code);
                    continue;
                }
            };
            let cached = if target == 0 {
                supplied.get(&intent.id).cloned()
            } else {
                None
            };
            match cached {
                Some(out) => rolled = Some(out),
                None => {
                    let Some(check) = txn.canon().checks.get(&check_id) else {
                        reject_check(
                            &mut rejected,
                            &mut notices,
                            &intent,
                            KernelReason::UnknownKey,
                        );
                        continue;
                    };
                    let Some(actor) = frozen.actors.get(&intent.actor_id) else {
                        reject_check(
                            &mut rejected,
                            &mut notices,
                            &intent,
                            KernelReason::UnknownKey,
                        );
                        continue;
                    };
                    match resolve_check(
                        check,
                        &txn.canon().meters,
                        &intent.actor_id,
                        actor,
                        &viewer,
                        target,
                        &rng,
                        &mut die_cursor,
                    ) {
                        Ok(out) => {
                            if target == 0 {
                                supplied.insert(intent.id.clone(), out.clone());
                            }
                            rolled = Some(out);
                        }
                        Err(code) => {
                            reject_check(&mut rejected, &mut notices, &intent, code);
                            continue;
                        }
                    }
                }
            }
        } else if txn.canon().needs_roll(&intent.move_name) {
            trace.push("resolve_check".to_string());
            let check_id = txn
                .canon()
                .moves
                .get(&intent.move_name)
                .and_then(|def| def.check.clone())
                .ok_or(KernelReason::UnknownKey)?;
            let viewer = ViewerId::from_kind(intent.actor_kind, &intent.actor_id)?;
            let check = txn
                .canon()
                .checks
                .get(&check_id)
                .ok_or(KernelReason::UnknownKey)?;
            let seed = seed_preimage(&shard_id, tick_index);
            let rng = blake2s_256(seed.as_bytes());
            let mut cursor = 0usize;
            let Some(actor) = frozen.actors.get(&intent.actor_id) else {
                rejected.push(Rejected {
                    id: intent.id,
                    reason: KernelReason::UnknownKey.jsonl_reason().to_string(),
                });
                continue;
            };
            if let Err(code) = resolve_check(
                check,
                &txn.canon().meters,
                &intent.actor_id,
                actor,
                &viewer,
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
        let outcome = rolled.as_ref().map(|out| out.success);
        let expanded = match expand_outcome(txn.canon(), &frozen, &intent, outcome) {
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
                }
                Err(KernelReason::Skipped) => rejected.push(Rejected {
                    id: intent.id.clone(),
                    reason: KernelReason::Skipped.jsonl_reason().to_string(),
                }),
                Err(KernelReason::Budget) => {
                    budget_stop = true;
                    if accepted == 0 {
                        rejected.push(Rejected {
                            id: intent.id.clone(),
                            reason: KernelReason::Budget.jsonl_reason().to_string(),
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
            keep_roll(&mut notices, &mut rolled, &intent.actor_id);
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
                    Err(KernelReason::Budget) => {
                        if accepted == 0 {
                            rejected.push(Rejected {
                                id: intent.id,
                                reason: KernelReason::Budget.jsonl_reason().to_string(),
                            });
                        } else {
                            keep_roll(&mut notices, &mut rolled, &intent.actor_id);
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
            keep_roll(&mut notices, &mut rolled, &intent.actor_id);
            applied.push(intent);
        }
    }

    if owns_commit {
        trace.push("tick_commit".to_string());
    }
    let jsonl = render_tick(
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
    );
    Ok((jsonl, notices))
}

/// The named actor's queued row, resolved once at target 0. Its total opposes this check.
fn supplied_target(
    canon: &spookery_kernel::canon::Canon,
    now: &spookery_kernel::model::Now,
    queued: &[Intent],
    intent: &Intent,
    rng: &[u8],
    die_cursor: &mut usize,
    supplied: &mut BTreeMap<String, CheckOut>,
) -> Result<Option<i64>, KernelReason> {
    let Target::Actor(named) = &intent.target else {
        return Ok(None);
    };
    let Some(opposing) = queued.iter().find(|other| {
        other.id != intent.id
            && other.actor_id == *named
            && other.source == Source::Player
            && canon.row_check(&other.move_name).is_some()
    }) else {
        return Ok(None);
    };
    if let Some(out) = supplied.get(&opposing.id) {
        return Ok(Some(out.total));
    }
    let Some(check_id) = canon.row_check(&opposing.move_name) else {
        return Ok(None);
    };
    let viewer = ViewerId::from_kind(opposing.actor_kind, &opposing.actor_id)?;
    let check = canon
        .checks
        .get(&check_id)
        .ok_or(KernelReason::UnknownKey)?;
    let actor = now
        .actors
        .get(&opposing.actor_id)
        .ok_or(KernelReason::UnknownKey)?;
    let out = resolve_check(
        check,
        &canon.meters,
        &opposing.actor_id,
        actor,
        &viewer,
        0,
        rng,
        die_cursor,
    )?;
    let total = out.total;
    supplied.insert(opposing.id.clone(), out);
    Ok(Some(total))
}

fn reject_check(
    rejected: &mut Vec<Rejected>,
    notices: &mut Vec<CheckNotice>,
    intent: &Intent,
    code: KernelReason,
) {
    rejected.push(Rejected {
        id: intent.id.clone(),
        reason: code.jsonl_reason().to_string(),
    });
    notices.push(CheckNotice::Refused {
        actor_id: intent.actor_id.clone(),
        code,
    });
}

fn keep_roll(notices: &mut Vec<CheckNotice>, rolled: &mut Option<CheckOut>, actor_id: &str) {
    if let Some(out) = rolled.take() {
        notices.push(CheckNotice::Rolled {
            actor_id: actor_id.to_string(),
            out,
        });
    }
}

fn viewer_token(intent: &Intent) -> Result<String, KernelReason> {
    match intent.actor_kind {
        ActorKind::Npc => Ok(format!("npc:{}", intent.actor_id)),
        ActorKind::Player => Ok(format!("player:{}", intent.actor_id)),
        ActorKind::System => Err(KernelReason::UnscopedPerceive),
    }
}
