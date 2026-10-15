use std::collections::BTreeSet;

use spookery_kernel::apply::apply_effect;
use spookery_kernel::expand::expand_rule;
use spookery_kernel::intent::{ActorKind, Intent, IntentArgs, Source, Target};
use spookery_kernel::key::{ChangeRecord, StateKey, StateValue};
use spookery_kernel::rules::cond;
use spookery_kernel::rules::expr::{self, Ctx};
use spookery_kernel::rules::rule::{ChangeWatch, Op, Owner, Rule, When};
use spookery_kernel::rules::RollReport;
use spookery_kernel::world::Draft;
use spookery_kernel::Reason;

use crate::resolve::{roll_subject, SubjectRoll};
use crate::round::Round;

/// Change rules, edge-triggered, to a fixpoint.
pub(crate) fn run(round: &mut Round, draft: &mut Draft) -> Result<(), Reason> {
    round.trace.push("run_triggers".to_string());
    let max = draft.budget().max_trigger_depth;
    let mut depth = 0u64;
    loop {
        let changes = draft.take_changes();
        if changes.is_empty() {
            break;
        }
        let hits = plan(draft, &changes);
        if hits.is_empty() {
            break;
        }
        if depth >= max {
            round.trace.push("trigger_depth".to_string());
            break;
        }
        depth += 1;
        for hit in hits {
            fire(round, draft, &hit.rule, &hit.subject)?;
        }
    }
    Ok(())
}

struct Hit {
    rule: Rule,
    subject: String,
}

fn plan(draft: &Draft, changes: &[ChangeRecord]) -> Vec<Hit> {
    let mut rules: Vec<Rule> = draft
        .canon()
        .rules
        .iter()
        .filter(|rule| matches!(rule.when, When::Change(_)))
        .cloned()
        .collect();
    rules.sort_by(|left, right| {
        left.owner
            .rank()
            .cmp(&right.owner.rank())
            .then(left.file.cmp(&right.file))
            .then(left.index.cmp(&right.index))
    });
    let mut hits = Vec::new();
    let mut seen = BTreeSet::new();
    for rule in rules {
        let When::Change(watch) = &rule.when else {
            continue;
        };
        let mut subjects = Vec::new();
        for change in changes {
            if !matches_watch(draft, &rule, watch, change) {
                continue;
            }
            let Some(subject) = subject_of(draft, &change.key) else {
                continue;
            };
            if !subjects.contains(&subject) {
                subjects.push(subject);
            }
        }
        subjects.sort();
        for subject in subjects {
            if seen.insert((rule.id.clone(), subject.clone())) {
                hits.push(Hit {
                    rule: rule.clone(),
                    subject,
                });
            }
        }
    }
    hits
}

fn fire(round: &mut Round, draft: &mut Draft, rule: &Rule, subject: &str) -> Result<bool, Reason> {
    let kind = match draft.now().actors.get(subject) {
        Some(actor) if actor.is_pc() => ActorKind::Player,
        Some(_) => ActorKind::Npc,
        None => return Ok(false),
    };
    let intent = intent_for(round, subject, kind, &rule.id);
    if cond::holds(
        draft.canon(),
        draft.now(),
        &Ctx { intent: &intent },
        &rule.if_,
    )
    .is_err()
    {
        return Ok(false);
    }
    let point = draft.savepoint();
    let now = draft.now().clone();
    let checks = draft.canon().checks.clone();
    let meters = draft.canon().meters.clone();
    let rule_id = rule.id.clone();
    let mut rolled = None;
    let mut resist_reports = Vec::new();
    let outcome = if let Some(spec) = &rule.roll {
        let ctx = Ctx { intent: &intent };
        let target = match expr::eval(draft.canon(), &now, &ctx, &spec.target) {
            Ok(target) => target,
            Err(_) => {
                draft.rollback(point);
                return Ok(false);
            }
        };
        let partial_at = match &spec.partial_at {
            Some(expr) => match expr::eval(draft.canon(), &now, &ctx, expr) {
                Ok(value) => Some(value),
                Err(_) => {
                    draft.rollback(point);
                    return Ok(false);
                }
            },
            None => None,
        };
        let Some(actor) = now.actors.get(subject).cloned() else {
            draft.rollback(point);
            return Ok(false);
        };
        match roll_subject(
            round,
            &checks,
            &meters,
            &SubjectRoll {
                rule_id: &rule_id,
                actor_kind: kind,
                actor_id: subject,
                check_id: &spec.check,
                target,
                partial_at,
                live: Some(&actor),
            },
        ) {
            Ok(out) => {
                let band = out.outcome;
                rolled = Some(out);
                Some(band)
            }
            Err(_) => {
                draft.rollback(point);
                return Ok(false);
            }
        }
    } else {
        None
    };
    let expanded = match expand_rule(
        draft.canon(),
        &now,
        rule,
        &intent,
        outcome,
        |actor_id, check_id, target| {
            let actor = now
                .actors
                .get(actor_id)
                .cloned()
                .ok_or(Reason::UnknownKey)?;
            let actor_kind = if actor.is_pc() {
                ActorKind::Player
            } else {
                ActorKind::Npc
            };
            let out = roll_subject(
                round,
                &checks,
                &meters,
                &SubjectRoll {
                    rule_id: &rule_id,
                    actor_kind,
                    actor_id,
                    check_id,
                    target,
                    partial_at: None,
                    live: Some(&actor),
                },
            )?;
            let band = out.outcome;
            resist_reports.push(RollReport::Rolled {
                actor_id: actor_id.to_string(),
                out,
            });
            Ok(band)
        },
    ) {
        Ok(effects) => effects,
        Err(_) => {
            draft.rollback(point);
            return Ok(false);
        }
    };
    if expanded.len() > draft.budget().max_effects_per_intent as usize {
        draft.rollback(point);
        return Ok(false);
    }
    for effect in &expanded {
        if apply_effect(draft, &intent, effect.clone()).is_err() {
            draft.rollback(point);
            return Ok(false);
        }
    }
    if let Some(out) = rolled {
        round.roll_reports.push(RollReport::Rolled {
            actor_id: subject.to_string(),
            out,
        });
    }
    round.roll_reports.extend(resist_reports);
    round.effects.extend(expanded);
    round.trace.push(rule_id);
    Ok(true)
}

fn intent_for(round: &Round, actor_id: &str, kind: ActorKind, rule_id: &str) -> Intent {
    Intent {
        id: format!("trigger:{rule_id}:{actor_id}"),
        tick_index: round.tick_index,
        actor_id: actor_id.to_string(),
        actor_kind: kind,
        source: Source::Kernel,
        move_name: rule_id.to_string(),
        target: Target::Empty,
        args: IntentArgs::default(),
        rationale_code: String::new(),
    }
}

fn subject_of(draft: &Draft, key: &StateKey) -> Option<String> {
    match key {
        StateKey::ClockValue(id) | StateKey::ClockPaused(id) => draft
            .now()
            .clocks
            .get(id)
            .and_then(|clock| {
                clock
                    .owners
                    .iter()
                    .find(|owner| draft.now().actors.contains_key(owner.as_str()))
            })
            .cloned(),
        other => other.actor().map(str::to_string),
    }
}

fn matches_watch(draft: &Draft, rule: &Rule, watch: &ChangeWatch, change: &ChangeRecord) -> bool {
    match watch {
        ChangeWatch::Arrive { place, who } => match &change.key {
            StateKey::ActorPlace(actor) => {
                let place = watched_place(rule, place);
                text(&change.before) != Some(place)
                    && text(&change.after) == Some(place)
                    && who_ok(draft, who, actor)
            }
            _ => false,
        },
        ChangeWatch::Leave { place, who } => match &change.key {
            StateKey::ActorPlace(actor) => {
                let place = watched_place(rule, place);
                text(&change.before) == Some(place)
                    && text(&change.after) != Some(place)
                    && who_ok(draft, who, actor)
            }
            _ => false,
        },
        ChangeWatch::Meter { key, crosses } => match &change.key {
            StateKey::ActorMeter(_, meter) if meter == key => {
                crossed(&change.before, &change.after, crosses)
            }
            _ => false,
        },
        ChangeWatch::Flag { key, becomes } => match &change.key {
            StateKey::ActorFlag(_, flag) if flag == key => {
                mark(&change.before) == Some(!becomes) && mark(&change.after) == Some(*becomes)
            }
            _ => false,
        },
        ChangeWatch::Clock { id } => match &change.key {
            StateKey::ClockValue(clock) if clock == id => {
                let Some(max) = draft.now().clocks.get(id).map(|clock| clock.max) else {
                    return false;
                };
                matches!(
                    (number(&change.before), number(&change.after)),
                    (Some(before), Some(after)) if before < max && after == max
                )
            }
            _ => false,
        },
        ChangeWatch::Item { id, gained } => match &change.key {
            StateKey::ActorItem(_, item) | StateKey::PlaceItem(_, item) if item == id => {
                match (number(&change.before), number(&change.after)) {
                    (Some(before), Some(after)) if *gained => after > before,
                    (Some(before), Some(after)) => after < before,
                    _ => false,
                }
            }
            _ => false,
        },
        ChangeWatch::Knows { fact } => match &change.key {
            StateKey::ActorKnownFact(_) => {
                listed(&change.after, fact) && !listed(&change.before, fact)
            }
            _ => false,
        },
    }
}

fn watched_place<'a>(rule: &'a Rule, place: &'a str) -> &'a str {
    if place == "here" {
        if let Owner::Place(id) = &rule.owner {
            return id;
        }
    }
    place
}

fn who_ok(draft: &Draft, who: &str, actor_id: &str) -> bool {
    let Some(actor) = draft.now().actors.get(actor_id) else {
        return false;
    };
    match who {
        "any" => true,
        "pc" => actor.is_pc(),
        "npc" => actor.is_npc(),
        other => other == actor_id,
    }
}

fn crossed(before: &StateValue, after: &StateValue, crosses: &str) -> bool {
    let (Some(before), Some(after)) = (number(before), number(after)) else {
        return false;
    };
    let Some((op, bound)) = parse_cross(crosses) else {
        return false;
    };
    !op.holds(before, bound) && op.holds(after, bound)
}

fn parse_cross(text: &str) -> Option<(Op, i64)> {
    let text = text.trim();
    let (op, rest) = if let Some(rest) = text.strip_prefix("<=") {
        (Op::Le, rest)
    } else if let Some(rest) = text.strip_prefix(">=") {
        (Op::Ge, rest)
    } else if let Some(rest) = text.strip_prefix("==") {
        (Op::Eq, rest)
    } else if let Some(rest) = text.strip_prefix('<') {
        (Op::Lt, rest)
    } else if let Some(rest) = text.strip_prefix('>') {
        (Op::Gt, rest)
    } else {
        return None;
    };
    Some((op, rest.trim().parse().ok()?))
}

fn text(value: &StateValue) -> Option<&str> {
    match value {
        StateValue::Text(text) => Some(text),
        _ => None,
    }
}

fn number(value: &StateValue) -> Option<i64> {
    match value {
        StateValue::Number(number) => Some(*number),
        _ => None,
    }
}

fn mark(value: &StateValue) -> Option<bool> {
    match value {
        StateValue::Mark(mark) => Some(*mark),
        _ => None,
    }
}

fn listed(value: &StateValue, fact: &str) -> bool {
    match value {
        StateValue::List(ids) => ids.iter().any(|id| id == fact),
        _ => false,
    }
}
