use std::collections::BTreeMap;

use spookery_kernel::api::check::{classify, CheckDef};
use spookery_kernel::api::meter::MeterDef;
use spookery_kernel::api::rules::expr::{self, Ctx};
use spookery_kernel::api::rules::gate::gate;

use spookery_kernel::api::intent::{ActorKind, Intent, Source, Target, ViewerId};
use spookery_kernel::api::perceive::perceive_at;
use spookery_kernel::api::rules::{roll, RollInput, RollKey, RollReport, RollResult};
use spookery_kernel::api::world::Draft;
use spookery_kernel::api::Reason;

use crate::state::{Resolution, Round};

pub(crate) fn run(round: &mut Round, draft: &mut Draft) -> Result<(), Reason> {
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
    let rule = match gate(draft.canon(), draft.now(), intent) {
        Ok(rule) => rule.clone(),
        Err(miss) => return Ok(refused(&miss.named(), None)),
    };
    if spookery_kernel::api::map::span(draft.canon(), draft.now(), intent, &rule) > 0 {
        return Ok(Resolution::Ready { rolled: None });
    }
    if let Some(roll) = rule.roll.clone() {
        round.trace.push("resolve_check".to_string());
        return roll_check(round, draft, intent, &roll);
    }
    Ok(Resolution::Ready { rolled: None })
}

fn roll_check(
    round: &mut Round,
    draft: &Draft,
    intent: &Intent,
    roll: &spookery_kernel::api::rules::rule::RollSpec,
) -> Result<Resolution, Reason> {
    if let Err(code) = ViewerId::from_kind(intent.actor_kind, &intent.actor_id) {
        return Ok(refused_check(intent, code));
    }
    let opposed = match supplied_target(round, draft, intent) {
        Ok(total) => total,
        Err(code) => return Ok(refused_check(intent, code)),
    };
    let ctx = Ctx { intent };
    let partial_at = if opposed.is_some() {
        None
    } else {
        match roll.partial_at.as_ref() {
            Some(expr) => Some(expr::eval(draft.canon(), &round.frozen, &ctx, expr)?),
            None => None,
        }
    };
    let target = match opposed {
        Some(total) => total,
        None => expr::eval(draft.canon(), &round.frozen, &ctx, &roll.target)?,
    };
    let check_id = roll.check.as_str();
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
                && gate(draft.canon(), draft.now(), other)
                    .ok()
                    .is_some_and(|rule| rule.roll.is_some())
        })
        .cloned();
    let Some(opposing) = opposing else {
        return Ok(None);
    };
    if let Some(out) = round.opposed.get(&opposing.id) {
        return Ok(Some(out.total));
    }
    let Some(check_id) = gate(draft.canon(), draft.now(), &opposing)
        .ok()
        .and_then(|rule| rule.roll.as_ref().map(|roll| roll.check.clone()))
    else {
        return Ok(None);
    };
    let out = roll_for(round, draft, &opposing, &check_id, 0, None)?;
    let total = out.total;
    round.opposed.insert(opposing.id, out);
    Ok(Some(total))
}

pub(crate) struct SubjectRoll<'a> {
    pub rule_id: &'a str,
    pub actor_kind: ActorKind,
    pub actor_id: &'a str,
    pub check_id: &'a str,
    pub target: i64,
    pub partial_at: Option<i64>,
    /// The actor at completion. Absent means the actor frozen at gather.
    pub live: Option<&'a spookery_kernel::api::model::ActorLive>,
}

/// The one `rules::roll` call. Action checks and resist rolls both come through here.
pub(crate) fn roll_subject(
    round: &mut Round,
    checks: &BTreeMap<String, CheckDef>,
    meters: &BTreeMap<String, MeterDef>,
    spec: &SubjectRoll<'_>,
) -> Result<RollResult, Reason> {
    let actor = match spec.live {
        Some(actor) => actor.clone(),
        None => round
            .frozen
            .actors
            .get(spec.actor_id)
            .cloned()
            .ok_or(Reason::UnknownKey)?,
    };
    let viewer = ViewerId::from_kind(spec.actor_kind, spec.actor_id)?;
    let check = checks.get(spec.check_id).ok_or(Reason::UnknownKey)?;
    let key = next_key(round, spec.rule_id, spec.actor_id);
    roll(
        RollInput {
            check,
            defs: meters,
            actor_id: spec.actor_id,
            actor: &actor,
            viewer: &viewer,
            target: spec.target,
            partial_at: spec.partial_at,
        },
        &key,
    )
}

fn roll_for(
    round: &mut Round,
    draft: &Draft,
    intent: &Intent,
    check_id: &str,
    target: i64,
    partial_at: Option<i64>,
) -> Result<RollResult, Reason> {
    let canon = draft.canon();
    roll_subject(
        round,
        &canon.checks,
        &canon.meters,
        &SubjectRoll {
            rule_id: &intent.move_name,
            actor_kind: intent.actor_kind,
            actor_id: &intent.actor_id,
            check_id,
            target,
            partial_at,
            live: None,
        },
    )
}

pub(crate) fn next_key(round: &mut Round, rule: &str, subject: &str) -> RollKey {
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
        round_index: round.tick_index,
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
