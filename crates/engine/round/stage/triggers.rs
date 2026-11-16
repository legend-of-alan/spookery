//! Change rules, edge-triggered, to a fixpoint, after each start and each finish lands.

use crate::canon::{Canon, OPEN};
use crate::ecs::change::{Change, ChangeValue};
use crate::ecs::component::{Component, Flag, Item, KnownFacts, Meter, OpenExit, Place};
use crate::ecs::id::ActorId;
use crate::ecs::{self, component, entity, Presence};
use crate::intent::Intent;
use crate::rules::cond;
use crate::rules::context::Context;
use crate::rules::expand::{self, Cause};
use crate::rules::gate;
use crate::rules::grammar;
use crate::rules::rule::{ChangeWatch, Op, Rule, When};
use crate::store::records::Step;

use crate::round::dice::RoundDice;
use crate::round::log::RoundLog;
use crate::round::stage::apply;

/// One trigger pass: fire the change rules the changes since the last pass match, and again on
/// what they write, at most `max_trigger_depth` times. Every change record goes on the log.
pub(crate) fn run(
    canon: &Canon,
    world: &mut ecs::World,
    presence: &Presence,
    dice: &mut RoundDice,
    log: &mut RoundLog,
) {
    let max = canon.budget.max_trigger_depth;
    let rules = change_rules(canon);
    let mut depth = 0u64;
    loop {
        let changes = world.take_changes();
        if changes.is_empty() {
            break;
        }
        log.changes.extend(changes.iter().cloned());
        let matched_triggers = match_rules(world, &rules, &changes);
        if matched_triggers.is_empty() {
            break;
        }
        if depth >= max {
            log.extra_steps.push(Step::TriggerDepth);
            break;
        }
        depth += 1;
        for matched in matched_triggers {
            let subject = matched.subject.as_ref();
            fire(canon, world, presence, dice, log, matched.rule, subject);
        }
    }
}

/// One change rule to fire, and the actor it fires for. A world change has none.
struct Trigger<'a> {
    rule: &'a Rule,
    subject: Option<ActorId>,
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
fn match_rules<'a>(world: &ecs::World, rules: &[&'a Rule], changes: &[Change]) -> Vec<Trigger<'a>> {
    let mut matched_triggers = Vec::new();
    for &rule in rules {
        let When::Change(watch) = &rule.when else {
            continue;
        };
        let mut subject_ids = Vec::new();
        for change in changes {
            if !matches_watch(world, rule, watch, change) {
                continue;
            }
            let subject = match &change.key.entity_id {
                entity::Id::Actor(actor) => Some(actor.clone()),
                _ => None,
            };
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
    canon: &Canon,
    world: &mut ecs::World,
    presence: &Presence,
    dice: &mut RoundDice,
    log: &mut RoundLog,
    rule: &Rule,
    subject: Option<&ActorId>,
) {
    let Ok(intent) = Intent::trigger(world, subject, &rule.id) else {
        return;
    };
    let context = Context::new(canon, world, presence, &intent);
    if cond::all_hold(&context, &rule.if_).is_err() {
        return;
    }
    let roller = dice.roller(log, &rule.id, subject);
    match expand::expand(&context, rule, Cause::Trigger, None, roller) {
        Ok(expansion) => {
            apply::land(canon, world, presence, &intent, rule, expansion, log);
        }
        Err(code) => log.refused.push(gate::Refusal::new(&intent, code)),
    }
}

fn matches_watch(world: &ecs::World, rule: &Rule, watch: &ChangeWatch, change: &Change) -> bool {
    let changed = &change.key;
    let (before, after) = (change.before.as_ref(), change.after.as_ref());
    match (watch, changed.component_id, &changed.entity_id) {
        (ChangeWatch::Arrive { place, subject }, Place::ID, entity::Id::Actor(actor)) => {
            let place = rule.watched_place(place);
            before.and_then(ChangeValue::as_text) != Some(place)
                && after.and_then(ChangeValue::as_text) == Some(place)
                && matches_subject(world, subject, actor)
        }
        (ChangeWatch::Leave { place, subject }, Place::ID, entity::Id::Actor(actor)) => {
            let place = rule.watched_place(place);
            before.and_then(ChangeValue::as_text) == Some(place)
                && after.and_then(ChangeValue::as_text) != Some(place)
                && matches_subject(world, subject, actor)
        }
        (ChangeWatch::Meter { key, op, bound }, Meter::ID, _)
            if changed.entry_id.as_ref() == Some(key) =>
        {
            crosses(before, after, *op, *bound)
        }
        (ChangeWatch::Flag { key, becomes }, Flag::ID, _)
            if changed.entry_id.as_ref() == Some(key) =>
        {
            flips(before, after, *becomes)
        }
        (ChangeWatch::Flag { key, becomes }, OpenExit::ID, _) if key == OPEN => {
            flips(before, after, *becomes)
        }
        (ChangeWatch::Item { id, gained }, Item::ID, entity::Id::Actor(_))
            if changed.entry_id.as_ref() == Some(id) =>
        {
            match (as_number(before), as_number(after)) {
                (Some(before), Some(after)) if *gained => after > before,
                (Some(before), Some(after)) => after < before,
                _ => false,
            }
        }
        (ChangeWatch::Knows { fact }, KnownFacts::ID, _) => {
            let lists_fact =
                |value: Option<&ChangeValue>| value.is_some_and(|ids| ids.contains(fact));
            lists_fact(after) && !lists_fact(before)
        }
        _ => false,
    }
}

/// Whether a flag change went from `!becomes` to `becomes`.
fn flips(before: Option<&ChangeValue>, after: Option<&ChangeValue>, becomes: bool) -> bool {
    before == Some(&ChangeValue::Bool(!becomes)) && after == Some(&ChangeValue::Bool(becomes))
}

fn matches_subject(world: &ecs::World, subject: &grammar::Subject, actor: &ActorId) -> bool {
    let Some(control_type) = world.actors().get::<component::ControlType>(actor).get() else {
        return false;
    };
    match subject {
        grammar::Subject::Any => true,
        grammar::Subject::Pc => control_type == component::ControlType::Pc,
        grammar::Subject::Npc => control_type == component::ControlType::Npc,
        grammar::Subject::Actor(expected) => expected == actor.as_str(),
    }
}

fn crosses(before: Option<&ChangeValue>, after: Option<&ChangeValue>, op: Op, bound: i64) -> bool {
    let (Some(before), Some(after)) = (as_number(before), as_number(after)) else {
        return false;
    };
    !op.holds(before, bound) && op.holds(after, bound)
}

/// A side of a change as a number: a meter's reading or an item's count.
fn as_number(value: Option<&ChangeValue>) -> Option<i64> {
    value?.as_number()
}
