//! Change rules, edge-triggered, to a fixpoint.

use crate::canon::Canon;
use crate::draft::Draft;
use crate::intent::{trigger_intent_id, Intent, Source, Target, Viewer};
use crate::journal::{Rejected, Step};
use crate::key::{Change, StateKey, StateValue};
use crate::rule::{ChangeWatch, Op, Rule, WatchWho, When};
use crate::rules::cond;
use crate::rules::expr::Ctx;
use crate::rules::MoveReport;
use crate::Reason;

use crate::round::dice::{Expansion, RoundDice};
use crate::round::log::RoundLog;

/// Every change record of the round, pass by pass.
pub(crate) fn run(
    draft: &mut Draft,
    dice: &mut RoundDice,
    log: &mut RoundLog,
) -> Result<Vec<Change>, Reason> {
    let max = draft.budget().max_trigger_depth;
    let canon = draft.shared_canon();
    let rules = change_rules(&canon);
    let mut depth = 0u64;
    let mut all_changes = Vec::new();
    loop {
        let changes = draft.take_changes();
        if changes.is_empty() {
            break;
        }
        all_changes.extend(changes.iter().cloned());
        let matched_triggers = match_rules(draft, &rules, &changes);
        if matched_triggers.is_empty() {
            break;
        }
        if depth >= max {
            log.steps.push(Step::TriggerDepth);
            break;
        }
        depth += 1;
        for matched in matched_triggers {
            fire(draft, dice, log, matched.rule, &matched.subject);
        }
    }
    Ok(all_changes)
}

struct Match<'a> {
    rule: &'a Rule,
    subject: String,
}

fn change_rules(canon: &Canon) -> Vec<&Rule> {
    let mut rules: Vec<&Rule> = canon
        .rules
        .iter()
        .filter(|rule| matches!(rule.when, When::Change(_)))
        .collect();
    rules.sort_by(|left, right| {
        left.owner
            .rank()
            .cmp(&right.owner.rank())
            .then(left.file.cmp(&right.file))
            .then(left.index.cmp(&right.index))
    });
    rules
}

/// Which rules a batch of changes triggers, and for whom.
fn match_rules<'a>(draft: &Draft, rules: &[&'a Rule], changes: &[Change]) -> Vec<Match<'a>> {
    let mut matched_triggers = Vec::new();
    for &rule in rules {
        let When::Change(watch) = &rule.when else {
            continue;
        };
        let mut subjects = Vec::new();
        for change in changes {
            if !matches_watch(draft, rule, watch, change) {
                continue;
            }
            for subject in subjects_of(draft, &change.key) {
                if !subjects.contains(&subject) {
                    subjects.push(subject);
                }
            }
        }
        subjects.sort();
        for subject in subjects {
            matched_triggers.push(Match { rule, subject });
        }
    }
    matched_triggers
}

fn fire(draft: &mut Draft, dice: &mut RoundDice, log: &mut RoundLog, rule: &Rule, subject: &str) {
    let Some(viewer) = draft
        .state()
        .actors
        .get(subject)
        .map(|actor| actor.viewer(subject))
    else {
        return;
    };
    let intent = trigger_intent(draft.state().round_index, viewer, &rule.id);
    if cond::holds(
        draft.canon(),
        draft.state(),
        &Ctx {
            intent: &intent,
            presence: draft.presence,
        },
        &rule.if_,
    )
    .is_err()
    {
        return;
    }
    let point = draft.savepoint();
    let expanded = expand_trigger(dice, log, draft, rule, &intent, subject);
    let landed = expanded.and_then(|expanded| {
        for effect in &expanded.effects {
            effect.clone().apply(draft, &intent)?;
        }
        Ok(expanded)
    });
    let expanded = match landed {
        Ok(expanded) => expanded,
        Err(code) => {
            draft.rollback(point);
            log.rejected.push(Rejected {
                id: intent.id,
                code,
                missing: None,
            });
            return;
        }
    };
    if let Some(out) = expanded.rolled {
        log.reports.push(MoveReport::Rolled {
            actor_id: subject.to_string(),
            roll: out,
        });
    }
    log.reports.extend(expanded.resisted);
    log.effects.extend(expanded.effects);
    log.steps.push(Step::Rule(rule.id.clone()));
}

/// A change rule's effects, or why it is refused. Over the effect cap is `BUDGET`.
fn expand_trigger(
    dice: &mut RoundDice,
    log: &mut RoundLog,
    draft: &Draft,
    rule: &Rule,
    intent: &Intent,
    subject: &str,
) -> Result<Expansion, Reason> {
    let (now, canon, presence) = (draft.state(), draft.canon(), draft.presence);
    let mut roller = dice.roller(log, &rule.id, subject);
    let rolled = match &rule.roll {
        Some(spec) => Some(roller.roll_spec(canon, now, presence, intent, spec)?),
        None => None,
    };
    let outcome = rolled.as_ref().map(|out| out.outcome);
    let effects = rule.expand(canon, now, presence, intent, outcome, &mut roller)?;
    if effects.len() > draft.budget().max_effects_per_intent as usize {
        return Err(Reason::Budget);
    }
    Ok(Expansion {
        effects,
        rolled,
        resisted: roller.into_reports(),
    })
}

fn trigger_intent(round_index: u64, actor: Viewer, rule_id: &str) -> Intent {
    Intent::new(
        trigger_intent_id(rule_id, actor.actor_id()),
        round_index,
        actor,
        Source::Kernel,
        rule_id,
        Target::Empty,
    )
}

fn subjects_of(draft: &Draft, key: &StateKey) -> Vec<String> {
    if let StateKey::WorldFlag(_) = key {
        return draft
            .presence
            .actors_present
            .iter()
            .filter(|id| draft.state().actors.contains_key(id.as_str()))
            .cloned()
            .collect();
    }
    subject_of(draft, key).into_iter().collect()
}

fn subject_of(draft: &Draft, key: &StateKey) -> Option<String> {
    match key {
        StateKey::ClockValue(id) | StateKey::ClockPaused(id) => draft
            .state()
            .clocks
            .get(id)
            .and_then(|clock| {
                clock
                    .owners
                    .iter()
                    .find(|owner| draft.state().actors.contains_key(owner.as_str()))
            })
            .cloned(),
        other => other.actor().map(str::to_string),
    }
}

fn matches_watch(draft: &Draft, rule: &Rule, watch: &ChangeWatch, change: &Change) -> bool {
    match watch {
        ChangeWatch::Arrive { place, who } => match &change.key {
            StateKey::ActorPlace(actor) => {
                let place = rule.watched_place(place);
                change.before.as_text() != Some(place)
                    && change.after.as_text() == Some(place)
                    && matches_who(draft, who, actor)
            }
            _ => false,
        },
        ChangeWatch::Leave { place, who } => match &change.key {
            StateKey::ActorPlace(actor) => {
                let place = rule.watched_place(place);
                change.before.as_text() == Some(place)
                    && change.after.as_text() != Some(place)
                    && matches_who(draft, who, actor)
            }
            _ => false,
        },
        ChangeWatch::Meter { key, op, bound } => match &change.key {
            StateKey::ActorMeter(_, meter) if meter == key => {
                crosses(&change.before, &change.after, *op, *bound)
            }
            _ => false,
        },
        ChangeWatch::Flag { key, becomes } => match &change.key {
            StateKey::ActorFlag(_, flag) | StateKey::WorldFlag(flag) if flag == key => {
                change.before.as_bool() == Some(!becomes)
                    && change.after.as_bool() == Some(*becomes)
            }
            _ => false,
        },
        ChangeWatch::Clock { id } => match &change.key {
            StateKey::ClockValue(clock) if clock == id => {
                let Some(max) = draft.state().clocks.get(id).map(|clock| clock.max) else {
                    return false;
                };
                matches!(
                    (change.before.as_number(), change.after.as_number()),
                    (Some(before), Some(after)) if before < max && after == max
                )
            }
            _ => false,
        },
        ChangeWatch::Item { id, gained } => match &change.key {
            StateKey::ActorItem(_, item) if item == id => {
                match (change.before.as_number(), change.after.as_number()) {
                    (Some(before), Some(after)) if *gained => after > before,
                    (Some(before), Some(after)) => after < before,
                    _ => false,
                }
            }
            _ => false,
        },
        ChangeWatch::Knows { fact } => match &change.key {
            StateKey::ActorKnownFact(_) => {
                change.after.contains(fact) && !change.before.contains(fact)
            }
            _ => false,
        },
    }
}

fn matches_who(draft: &Draft, who: &WatchWho, actor_id: &str) -> bool {
    let Some(actor) = draft.state().actors.get(actor_id) else {
        return false;
    };
    match who {
        WatchWho::Any => true,
        WatchWho::Pc => actor.is_pc(),
        WatchWho::Npc => actor.is_npc(),
        WatchWho::Actor(expected) => expected == actor_id,
    }
}

fn crosses(before: &StateValue, after: &StateValue, op: Op, bound: i64) -> bool {
    let (Some(before), Some(after)) = (before.as_number(), after.as_number()) else {
        return false;
    };
    !op.holds(before, bound) && op.holds(after, bound)
}
