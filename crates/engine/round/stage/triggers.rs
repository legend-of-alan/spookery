//! Change rules, edge-triggered, to a fixpoint.

use crate::canon::{Canon, OPEN};
use crate::draft::Draft;
use crate::intent::Intent;
use crate::model::key::{Change, StateKey, StateValue};
use crate::rules::cond;
use crate::rules::context::Context;
use crate::rules::expand::{self, Cause};
use crate::rules::gate;
use crate::rules::rule::{ChangeWatch, Op, Rule, WatchWho, When};
use crate::store::records::Step;
use crate::Reason;

use crate::round::dice::RoundDice;
use crate::round::log::RoundLog;
use crate::round::stage::apply;

/// Every change record of the round, pass by pass.
pub(crate) fn run(
    draft: &mut Draft,
    canon: &Canon,
    dice: &mut RoundDice,
    log: &mut RoundLog,
) -> Result<Vec<Change>, Reason> {
    let max = draft.budget().max_trigger_depth;
    let rules = change_rules(canon);
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
            log.extra_steps.push(Step::TriggerDepth);
            break;
        }
        depth += 1;
        for matched in matched_triggers {
            fire(draft, dice, log, matched.rule, matched.subject.as_deref());
        }
    }
    Ok(all_changes)
}

/// One change rule to fire, and the actor it fires for. A world change has none.
struct Trigger<'a> {
    rule: &'a Rule,
    subject: Option<String>,
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
fn match_rules<'a>(draft: &Draft, rules: &[&'a Rule], changes: &[Change]) -> Vec<Trigger<'a>> {
    let mut matched_triggers = Vec::new();
    for &rule in rules {
        let When::Change(watch) = &rule.when else {
            continue;
        };
        let mut subject_ids = Vec::new();
        for change in changes {
            if !matches_watch(draft, rule, watch, change) {
                continue;
            }
            let subject = change.key.actor().map(str::to_string);
            if !subject_ids.contains(&subject) {
                subject_ids.push(subject);
            }
        }
        subject_ids.sort();
        for subject in subject_ids {
            matched_triggers.push(Trigger { rule, subject });
        }
    }
    matched_triggers
}

/// Run a change rule for `subject`, or for no one on a world change, when its `if` holds. A
/// refusal is journaled and told to nobody.
fn fire(
    draft: &mut Draft,
    dice: &mut RoundDice,
    log: &mut RoundLog,
    rule: &Rule,
    subject: Option<&str>,
) {
    let Ok(intent) = Intent::trigger(draft.state(), subject, &rule.id) else {
        return;
    };
    let context = Context::new(draft.canon(), draft.state(), draft.presence, &intent);
    if cond::all_hold(&context, &rule.if_).is_err() {
        return;
    }
    let roller = dice.roller(log, &rule.id, subject);
    match expand::expand(&context, rule, Cause::Trigger, None, roller) {
        Ok(expansion) => {
            apply::land(draft, &intent, rule, expansion, log);
        }
        Err(code) => log.refused.push(gate::Refusal::new(&intent, code)),
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
            StateKey::ActorMeter(_, meter)
            | StateKey::PlaceMeter(_, meter)
            | StateKey::StoryMeter(_, meter)
            | StateKey::WorldMeter(meter)
                if meter == key =>
            {
                crosses(&change.before, &change.after, *op, *bound)
            }
            _ => false,
        },
        ChangeWatch::Flag { key, becomes } => match &change.key {
            StateKey::ActorFlag(_, flag)
            | StateKey::PlaceFlag(_, flag)
            | StateKey::StoryFlag(_, flag)
            | StateKey::WorldFlag(flag)
                if flag == key =>
            {
                flips(change, *becomes)
            }
            StateKey::ExitOpen(_, _) if key == OPEN => flips(change, *becomes),
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

/// Whether a flag change went from `!becomes` to `becomes`.
fn flips(change: &Change, becomes: bool) -> bool {
    change.before.as_bool() == Some(!becomes) && change.after.as_bool() == Some(becomes)
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
