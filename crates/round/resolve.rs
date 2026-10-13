use spookery_kernel::check::classify;
use spookery_kernel::habit::allowed;
use spookery_kernel::intent::{ActorKind, Intent, Source, Target, ViewerId};
use spookery_kernel::perceive::perceive_at;
use spookery_kernel::rules::{roll, RollInput, RollKey, RollReport, RollResult};
use spookery_kernel::world::Draft;
use spookery_kernel::Reason;

use crate::round::{Resolution, Round};

pub fn resolve(round: &mut Round, draft: &mut Draft) -> Result<(), Reason> {
    round.trace.push("resolve".to_string());
    round.frozen = draft.now().clone();
    let candidates = round.candidates.clone();
    for intent in candidates {
        let resolution = consider(round, draft, &intent)?;
        round.resolutions.insert(intent.id, resolution);
    }
    Ok(())
}

fn consider(round: &mut Round, draft: &Draft, intent: &Intent) -> Result<Resolution, Reason> {
    round.trace.push(format!("perceive:{}", intent.actor_id));
    let view = perceive_at(draft.canon(), draft.now(), &viewer_token(intent)?)?;
    if view.known_facts.is_empty() {
        return Ok(refused("empty_perceive", None));
    }
    if let Err(code) = allowed(draft.canon(), draft.now(), intent) {
        return Ok(refused(code.jsonl_reason(), None));
    }
    if let Some(check_id) = draft.canon().row_check(&intent.move_name) {
        round.trace.push("resolve_check".to_string());
        return roll_row(round, draft, intent, &check_id);
    }
    if draft.canon().needs_roll(&intent.move_name) {
        round.trace.push("resolve_check".to_string());
        return validate_roll(round, draft, intent);
    }
    Ok(Resolution::Ready { rolled: None })
}

fn roll_row(
    round: &mut Round,
    draft: &Draft,
    intent: &Intent,
    check_id: &str,
) -> Result<Resolution, Reason> {
    if let Err(code) = ViewerId::from_kind(intent.actor_kind, &intent.actor_id) {
        return Ok(refused_check(intent, code));
    }
    let opposed = match supplied_target(round, draft, intent) {
        Ok(total) => total,
        Err(code) => return Ok(refused_check(intent, code)),
    };
    let partial_at = if opposed.is_some() {
        None
    } else {
        draft.canon().row_partial_at(&intent.move_name)
    };
    let target = opposed.unwrap_or_else(|| draft.canon().row_difficulty(&intent.move_name));
    if let Some(out) = round.opposed.get(&intent.id).cloned() {
        return Ok(Resolution::Ready {
            rolled: Some(reclassify(&out, target, partial_at)),
        });
    }
    match roll_for(round, draft, intent, check_id, target, partial_at) {
        Ok(out) => {
            round.opposed.insert(intent.id.clone(), out.clone());
            Ok(Resolution::Ready { rolled: Some(out) })
        }
        Err(code) => Ok(refused_check(intent, code)),
    }
}

/// A move that names a check and has no row. The roll is discarded; only a refusal matters.
fn validate_roll(round: &mut Round, draft: &Draft, intent: &Intent) -> Result<Resolution, Reason> {
    let check_id = draft
        .canon()
        .moves
        .get(&intent.move_name)
        .and_then(|def| def.check.clone())
        .ok_or(Reason::UnknownKey)?;
    ViewerId::from_kind(intent.actor_kind, &intent.actor_id)?;
    if !draft.canon().checks.contains_key(&check_id) {
        return Err(Reason::UnknownKey);
    }
    if !round.frozen.actors.contains_key(&intent.actor_id) {
        return Ok(refused(Reason::UnknownKey.jsonl_reason(), None));
    }
    match roll_for(round, draft, intent, &check_id, 0, None) {
        Ok(_) => Ok(Resolution::Ready { rolled: None }),
        Err(code) => Ok(refused(code.jsonl_reason(), None)),
    }
}

fn supplied_target(
    round: &mut Round,
    draft: &Draft,
    intent: &Intent,
) -> Result<Option<i64>, Reason> {
    let Target::Actor(named) = &intent.target else {
        return Ok(None);
    };
    let named = named.clone();
    let opposing = round
        .supply_from
        .iter()
        .find(|other| {
            other.id != intent.id
                && other.actor_id == named
                && other.source == Source::Player
                && draft.canon().row_check(&other.move_name).is_some()
        })
        .cloned();
    let Some(opposing) = opposing else {
        return Ok(None);
    };
    if let Some(out) = round.opposed.get(&opposing.id) {
        return Ok(Some(out.total));
    }
    let Some(check_id) = draft.canon().row_check(&opposing.move_name) else {
        return Ok(None);
    };
    let out = roll_for(round, draft, &opposing, &check_id, 0, None)?;
    let total = out.total;
    round.opposed.insert(opposing.id, out);
    Ok(Some(total))
}

fn roll_for(
    round: &mut Round,
    draft: &Draft,
    intent: &Intent,
    check_id: &str,
    target: i64,
    partial_at: Option<i64>,
) -> Result<RollResult, Reason> {
    let viewer = ViewerId::from_kind(intent.actor_kind, &intent.actor_id)?;
    let check = draft
        .canon()
        .checks
        .get(check_id)
        .cloned()
        .ok_or(Reason::UnknownKey)?;
    let actor = round
        .frozen
        .actors
        .get(&intent.actor_id)
        .cloned()
        .ok_or(Reason::UnknownKey)?;
    let key = next_key(round, &intent.move_name, &intent.actor_id);
    roll(
        RollInput {
            check: &check,
            defs: &draft.canon().meters,
            actor_id: &intent.actor_id,
            actor: &actor,
            viewer: &viewer,
            target,
            partial_at,
        },
        &key,
    )
}

fn next_key(round: &mut Round, rule: &str, subject: &str) -> RollKey {
    let roll_index = {
        let index = round
            .roll_counts
            .entry((rule.to_string(), subject.to_string()))
            .or_insert(0);
        let roll_index = *index;
        *index += 1;
        roll_index
    };
    RollKey {
        world_id: round.world_id.clone(),
        tick_index: round.tick_index,
        rule_id: rule.to_string(),
        subject_id: subject.to_string(),
        roll_index,
    }
}

fn reclassify(out: &RollResult, target: i64, partial_at: Option<i64>) -> RollResult {
    RollResult {
        faces: out.faces.clone(),
        stat: out.stat,
        total: out.total,
        target,
        outcome: classify(out.total, target, partial_at),
    }
}

fn refused(reason: &str, notice: Option<RollReport>) -> Resolution {
    Resolution::Refused {
        reason: reason.to_string(),
        notice,
    }
}

fn refused_check(intent: &Intent, code: Reason) -> Resolution {
    refused(
        code.jsonl_reason(),
        Some(RollReport::Refused {
            actor_id: intent.actor_id.clone(),
            code,
        }),
    )
}

pub(crate) fn viewer_token(intent: &Intent) -> Result<String, Reason> {
    match intent.actor_kind {
        ActorKind::Npc => Ok(format!("npc:{}", intent.actor_id)),
        ActorKind::Player => Ok(format!("player:{}", intent.actor_id)),
        ActorKind::System => Err(Reason::UnscopedPerceive),
    }
}
