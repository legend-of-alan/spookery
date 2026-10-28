//! Change rules, edge-triggered, to a fixpoint.

use crate::apply::apply_effect;
use crate::canon::Canon;
use crate::draft::Draft;
use crate::effect::Effect;
use crate::expand::expand_rule;
use crate::intent::{ActorKind, Intent, IntentArgs, Source, Target};
use crate::key::{ChangeRecord, StateKey, StateValue};
use crate::model::Now;
use crate::rule::{ChangeWatch, Op, Rule, WatchWho, When};
use crate::rules::cond;
use crate::rules::expr::Ctx;
use crate::rules::{MoveReport, RollResult};
use crate::Reason;

use crate::round::dice::{ResistKey, RoundDice};
use crate::round::log::RoundLog;

/// Every change record of the round, pass by pass.
pub(crate) fn run(
    draft: &mut Draft,
    dice: &mut RoundDice,
    log: &mut RoundLog,
) -> Result<Vec<ChangeRecord>, Reason> {
    let max = draft.budget().max_trigger_depth;
    let rules = change_rules(draft);
    let mut depth = 0u64;
    let mut all_changes = Vec::new();
    loop {
        let changes = draft.take_changes();
        if changes.is_empty() {
            break;
        }
        all_changes.extend(changes.iter().cloned());
        let matched_triggers = plan(draft, &rules, &changes);
        if matched_triggers.is_empty() {
            break;
        }
        if depth >= max {
            log.steps.push("trigger_depth".to_string());
            break;
        }
        depth += 1;
        for matched in matched_triggers {
            fire(draft, dice, log, matched.rule, &matched.subject);
        }
    }
    Ok(all_changes)
}

struct TriggerMatch<'a> {
    rule: &'a Rule,
    subject: String,
}

fn change_rules(draft: &Draft) -> Vec<Rule> {
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
    rules
}

fn plan<'a>(draft: &Draft, rules: &'a [Rule], changes: &[ChangeRecord]) -> Vec<TriggerMatch<'a>> {
    let mut matched_triggers = Vec::new();
    for rule in rules {
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
            matched_triggers.push(TriggerMatch { rule, subject });
        }
    }
    matched_triggers
}

struct TriggerExpansion {
    effects: Vec<Effect>,
    rolled: Option<RollResult>,
    resist: Vec<MoveReport>,
}

fn fire(draft: &mut Draft, dice: &mut RoundDice, log: &mut RoundLog, rule: &Rule, subject: &str) {
    let Some(kind) = draft
        .now()
        .actors
        .get(subject)
        .map(|actor| actor.actor_kind())
    else {
        return;
    };
    let intent = intent_for(draft.now().round_index, subject, kind, &rule.id);
    if cond::holds(
        draft.canon(),
        draft.now(),
        &Ctx { intent: &intent },
        &rule.if_,
    )
    .is_err()
    {
        return;
    }
    let point = draft.savepoint();
    let expanded = {
        let now = draft.now();
        let canon = draft.canon();
        expand_trigger(dice, log, now, canon, rule, &intent, subject)
    };
    let Some(expanded) = expanded else {
        draft.rollback(point);
        return;
    };
    if expanded.effects.len() > draft.budget().max_effects_per_intent as usize {
        draft.rollback(point);
        return;
    }
    for effect in &expanded.effects {
        if apply_effect(draft, &intent, effect.clone()).is_err() {
            draft.rollback(point);
            return;
        }
    }
    if let Some(out) = expanded.rolled {
        log.reports.push(MoveReport::Rolled {
            actor_id: subject.to_string(),
            out,
        });
    }
    log.reports.extend(expanded.resist);
    log.effects.extend(expanded.effects);
    log.steps.push(rule.id.clone());
}

fn expand_trigger(
    dice: &mut RoundDice,
    log: &mut RoundLog,
    now: &Now,
    canon: &Canon,
    rule: &Rule,
    intent: &Intent,
    subject: &str,
) -> Option<TriggerExpansion> {
    let mut roller = dice.roller(log, &rule.id, subject, ResistKey::Rule);
    let rolled = match &rule.roll {
        Some(spec) => Some(roller.roll_spec(canon, now, intent, spec, &rule.id).ok()?),
        None => None,
    };
    let outcome = rolled.as_ref().map(|out| out.outcome);
    let effects = expand_rule(canon, now, rule, intent, outcome, &mut roller).ok()?;
    Some(TriggerExpansion {
        effects,
        rolled,
        resist: roller.finish(),
    })
}

fn intent_for(round_index: u64, actor_id: &str, kind: ActorKind, rule_id: &str) -> Intent {
    Intent {
        id: format!("trigger:{rule_id}:{actor_id}"),
        round_index,
        actor_id: actor_id.to_string(),
        actor_kind: kind,
        source: Source::Kernel,
        move_name: rule_id.to_string(),
        target: Target::Empty,
        args: IntentArgs::default(),
        rationale_code: String::new(),
    }
}

fn subjects_of(draft: &Draft, key: &StateKey) -> Vec<String> {
    if let StateKey::WorldFlag(_) = key {
        return draft
            .now()
            .presence
            .actors_present
            .iter()
            .filter(|id| draft.now().actors.contains_key(id.as_str()))
            .cloned()
            .collect();
    }
    subject_of(draft, key).into_iter().collect()
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
                let place = rule.watched_place(place);
                text(&change.before) != Some(place)
                    && text(&change.after) == Some(place)
                    && who_ok(draft, who, actor)
            }
            _ => false,
        },
        ChangeWatch::Leave { place, who } => match &change.key {
            StateKey::ActorPlace(actor) => {
                let place = rule.watched_place(place);
                text(&change.before) == Some(place)
                    && text(&change.after) != Some(place)
                    && who_ok(draft, who, actor)
            }
            _ => false,
        },
        ChangeWatch::Meter { key, op, bound } => match &change.key {
            StateKey::ActorMeter(_, meter) if meter == key => {
                crossed(&change.before, &change.after, *op, *bound)
            }
            _ => false,
        },
        ChangeWatch::Flag { key, becomes } => match &change.key {
            StateKey::ActorFlag(_, flag) | StateKey::WorldFlag(flag) if flag == key => {
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
            StateKey::ActorItem(_, item) if item == id => {
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

fn who_ok(draft: &Draft, who: &WatchWho, actor_id: &str) -> bool {
    let Some(actor) = draft.now().actors.get(actor_id) else {
        return false;
    };
    match who {
        WatchWho::Any => true,
        WatchWho::Pc => actor.is_pc(),
        WatchWho::Npc => actor.is_npc(),
        WatchWho::Actor(expected) => expected == actor_id,
    }
}

fn crossed(before: &StateValue, after: &StateValue, op: Op, bound: i64) -> bool {
    let (Some(before), Some(after)) = (number(before), number(after)) else {
        return false;
    };
    !op.holds(before, bound) && op.holds(after, bound)
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
