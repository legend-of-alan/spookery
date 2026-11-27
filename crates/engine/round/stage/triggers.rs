//! Change rules, edge-triggered, to a fixpoint, after each start and each finish lands.

use crate::canon::Canon;
use crate::ecs::change::{Change, ChangeValue};
use crate::ecs::component::{Component, Flag, Item, Meter, Place};
use crate::ecs::id::ActorId;
use crate::ecs::{self, component, entity, Presence};
use crate::intent::Intent;
use crate::rules::cond;
use crate::rules::context::Context;
use crate::rules::expand::{self, Cause};
use crate::rules::gate;
use crate::rules::grammar;
use crate::rules::rule::{self, ChangeWatch, Rule};
use crate::store::records::Step;

use crate::round::stage::apply;
use crate::round::{self, Ledger};

/// One trigger pass: fire the change rules the changes since the last pass match, and again on
/// what they write, at most `max_trigger_depth` times. Every change record goes on the ledger.
pub(crate) fn run(
    canon: &Canon,
    world: &mut ecs::World,
    presence: &Presence,
    dice: &mut round::Dice,
    ledger: &mut Ledger,
) {
    let max = canon.budget.max_trigger_depth;
    let rules = change_rules(canon);
    let mut depth = 0u64;
    loop {
        let changes = world.take_changes();
        if changes.is_empty() {
            break;
        }
        ledger.changes.extend(changes.iter().cloned());
        let matched_triggers = match_rules(world, &rules, &changes);
        if matched_triggers.is_empty() {
            break;
        }
        if depth >= max {
            ledger.extra_steps.push(Step::TriggerDepth);
            break;
        }
        depth += 1;
        for matched in matched_triggers {
            let subject = matched.subject.as_ref();
            fire(canon, world, presence, dice, ledger, matched.rule, subject);
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
        .filter(|rule| matches!(rule.kind, rule::Kind::Change(_)))
        .collect();
    rules.sort_by(|left, right| {
        left.scope
            .rank()
            .cmp(&right.scope.rank())
            .then(left.file.cmp(&right.file))
            .then(left.index.cmp(&right.index))
    });
    rules
}

/// Which rules a batch of changes triggers, and for whom.
fn match_rules<'a>(world: &ecs::World, rules: &[&'a Rule], changes: &[Change]) -> Vec<Trigger<'a>> {
    let mut matched_triggers = Vec::new();
    for &rule in rules {
        let rule::Kind::Change(watch) = &rule.kind else {
            continue;
        };
        let mut subject_ids = Vec::new();
        for change in changes {
            if !matches_watch(world, watch, change) {
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
    dice: &mut round::Dice,
    ledger: &mut Ledger,
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
    let roller = dice.roller(ledger, &rule.id, subject);
    match expand::expand(&context, rule, Cause::Trigger, roller) {
        Ok(expansion) => {
            apply::land(canon, world, presence, &intent, rule, expansion, ledger);
        }
        Err(code) => ledger.refused.push(gate::Refusal::new(&intent, code)),
    }
}

fn matches_watch(world: &ecs::World, watch: &ChangeWatch, change: &Change) -> bool {
    let changed = &change.key;
    let (before, after) = (change.before.as_ref(), change.after.as_ref());
    match (watch, changed.component_id, &changed.entity_id) {
        (ChangeWatch::Arrive { place_id, subject }, Place::ID, entity::Id::Actor(actor_id)) => {
            before.and_then(ChangeValue::as_text) != Some(place_id.as_str())
                && after.and_then(ChangeValue::as_text) == Some(place_id.as_str())
                && matches_subject(world, subject, actor_id)
        }
        (ChangeWatch::Leave { place_id, subject }, Place::ID, entity::Id::Actor(actor_id)) => {
            before.and_then(ChangeValue::as_text) == Some(place_id.as_str())
                && after.and_then(ChangeValue::as_text) != Some(place_id.as_str())
                && matches_subject(world, subject, actor_id)
        }
        (ChangeWatch::Meter { id, op, bound }, Meter::ID, _)
            if changed.entry_id.as_ref() == Some(id) =>
        {
            crosses(before, after, *op, *bound)
        }
        (ChangeWatch::Flag { id, becomes }, Flag::ID, _)
            if changed.entry_id.as_ref() == Some(id) =>
        {
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
        grammar::Subject::Id(expected) => expected == actor.as_str(),
    }
}

fn crosses(
    before: Option<&ChangeValue>,
    after: Option<&ChangeValue>,
    op: cond::Op,
    bound: i64,
) -> bool {
    let (Some(before), Some(after)) = (as_number(before), as_number(after)) else {
        return false;
    };
    !op.holds(before, bound) && op.holds(after, bound)
}

/// A side of a change as a number: a meter's reading or an item's count.
fn as_number(value: Option<&ChangeValue>) -> Option<i64> {
    value?.as_number()
}
