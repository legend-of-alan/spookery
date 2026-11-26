//! Which change rules set each other's watched values, and who their narrates and notes reach.

use crate::canon::map::{PathDef, OPEN};
use crate::canon::Canon;
use crate::diagnostic::{error, schema_error, warning, Diagnostic};
use crate::ecs::component::meter;
use crate::rules::grammar;
use crate::rules::rule::{ChangeWatch, EffectTemplate, LocationTo, NarrateTo, Rule, When};
use crate::Reason;

/// Change rules that set each other's watched values.
pub(super) fn find_trigger_cycles(canon: &Canon) -> Vec<Diagnostic> {
    let rules: Vec<&Rule> = canon
        .rules
        .iter()
        .filter(|rule| matches!(rule.when, When::Change(_)))
        .collect();
    let written: Vec<Vec<EffectWrite>> = rules
        .iter()
        .map(|rule| {
            let mut writes = Vec::new();
            rule.visit_templates(|effect| push_writes(canon, effect, &mut writes));
            writes
        })
        .collect();
    let mut next = vec![Vec::new(); rules.len()];
    for (from, writes) in written.iter().enumerate() {
        for (to, rule) in rules.iter().enumerate() {
            if from == to {
                continue;
            }
            let When::Change(watch) = &rule.when else {
                continue;
            };
            if writes.iter().any(|write| sets_watch(watch, rule, write)) {
                next[from].push(to);
            }
        }
    }
    let mut in_cycle = vec![false; rules.len()];
    mark_cycles(&next, &mut in_cycle);
    rules
        .iter()
        .enumerate()
        .filter(|(index, _)| in_cycle[*index])
        .map(|(_, rule)| {
            warning(
                &rule.file,
                &format!("rule '{}' is in a trigger cycle", rule.local_id),
                "stop one rule in the cycle from setting the next watched value",
            )
            .at_path(format!("rules[{}]", rule.index))
        })
        .collect()
}

/// Narrate and note on a change rule that can reach a viewer who cannot see the change.
pub(super) fn find_trigger_leaks(canon: &Canon) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for rule in canon.rules.iter() {
        let When::Change(watch) = &rule.when else {
            continue;
        };
        let sight = sight_of(canon, watch);
        let mut leak = false;
        rule.visit_templates(|effect| {
            if reach_of(effect).is_some_and(|reach| is_too_wide(&sight, &reach)) {
                leak = true;
            }
        });
        if !leak {
            continue;
        }
        diagnostics.push(
            error(
                &rule.file,
                Reason::TriggerLeak,
                &format!(
                    "rule '{}' leaks a change to a viewer who cannot see it",
                    rule.local_id
                ),
                "address that output to self, or to here when the room can see the change",
            )
            .at_path(format!("rules[{}]", rule.index)),
        );
    }
    diagnostics
}

/// A value an effect writes, as a change watch matches it.
#[derive(Clone)]
enum EffectWrite {
    Flag { key: String, value: bool },
    Meter(String),
    Item { id: String, gained: bool },
    Arrive(String),
}

/// Who a narrate or note reaches: the subject, the place, or anyone else it names.
enum Reach {
    Subject,
    Place,
    Others,
}

/// Who may see a watched change. A meter or flag change on a watch counts as the place's,
/// because a rule that narrates it is the pack choosing to tell the place. A world change has
/// no place, so the pack tells whoever it names.
enum Sight {
    Place,
    World,
}

fn push_writes(canon: &Canon, effect: &EffectTemplate, out: &mut Vec<EffectWrite>) {
    match effect {
        EffectTemplate::Modifier { id, .. } => {
            if let Some(def) = canon.modifiers.get(id) {
                out.push(EffectWrite::Meter(def.meter.clone()));
            }
        }
        EffectTemplate::Flag { key, value, .. } => {
            out.push(EffectWrite::Flag {
                key: key.clone(),
                value: *value,
            });
        }
        EffectTemplate::Meter { key, .. } => out.push(EffectWrite::Meter(key.clone())),
        EffectTemplate::Item { id, add, .. } => {
            let constant_sign = add.constant().map(i64::signum);
            for (gained, sign) in [(true, 1), (false, -1)] {
                if constant_sign.is_none_or(|constant| constant == sign) {
                    out.push(EffectWrite::Item {
                        id: id.clone(),
                        gained,
                    });
                }
            }
        }
        EffectTemplate::Location {
            to: LocationTo::Place(id),
            ..
        } => out.push(EffectWrite::Arrive(id.clone())),
        _ => {}
    }
}

fn sets_watch(watch: &ChangeWatch, rule: &Rule, write: &EffectWrite) -> bool {
    match (watch, write) {
        (
            ChangeWatch::Flag { key, becomes },
            EffectWrite::Flag {
                key: written,
                value,
            },
        ) => key == written && becomes == value,
        (ChangeWatch::Meter { key, .. }, EffectWrite::Meter(written)) => key == written,
        (
            ChangeWatch::Item { id, gained },
            EffectWrite::Item {
                id: written,
                gained: up,
            },
        ) => id == written && gained == up,
        (ChangeWatch::Arrive { place, .. }, EffectWrite::Arrive(dest)) => {
            rule.watched_place(place) == dest
        }
        _ => false,
    }
}

fn sight_of(canon: &Canon, watch: &ChangeWatch) -> Sight {
    if watches_world(canon, watch) && !watches_actor_key(canon, watch) {
        Sight::World
    } else {
        Sight::Place
    }
}

fn reach_of(effect: &EffectTemplate) -> Option<Reach> {
    match effect {
        EffectTemplate::Narrate { to, .. } => Some(match to {
            NarrateTo::Self_ => Reach::Subject,
            NarrateTo::Here => Reach::Place,
            NarrateTo::Actors(_) | NarrateTo::Present => Reach::Others,
        }),
        EffectTemplate::Note { .. } => Some(Reach::Others),
        _ => None,
    }
}

fn is_too_wide(sight: &Sight, reach: &Reach) -> bool {
    match sight {
        Sight::Place => !matches!(reach, Reach::Subject | Reach::Place),
        Sight::World => false,
    }
}

/// Whether `watch` can match a change no actor holds, which has no subject: a meter the world,
/// a place or a story carries, or a flag some rule sets on the world, a place, a story or an
/// exit.
fn watches_world(canon: &Canon, watch: &ChangeWatch) -> bool {
    match watch {
        ChangeWatch::Meter { key, .. } => {
            canon.world_meters.contains_key(key)
                || canon
                    .meters
                    .get(key)
                    .is_some_and(|def| !def.is_actor_meter())
        }
        ChangeWatch::Flag { key, .. } => {
            (key == OPEN && canon.paths.iter().any(PathDef::is_conditional))
                || super::find_flag_objects(canon, key)
                    .any(|of| !matches!(of, grammar::Object::Actor(_)))
        }
        _ => false,
    }
}

/// Whether an actor may also hold the meter or flag `watch` names, so the watch can match an
/// actor's change as well as the world's.
fn watches_actor_key(canon: &Canon, watch: &ChangeWatch) -> bool {
    match watch {
        ChangeWatch::Meter { key, .. } => canon
            .meters
            .get(key)
            .is_some_and(meter::Def::is_actor_meter),
        ChangeWatch::Flag { key, .. } => {
            super::find_flag_objects(canon, key).any(|of| matches!(of, grammar::Object::Actor(_)))
        }
        _ => false,
    }
}

/// A change rule on a world change runs once, for no one, so it has no `self` and no `here`.
/// Every other rule has a subject, so its `narrate` names who hears it.
pub(super) fn find_world_change_subjects(canon: &Canon) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for rule in canon.rules.iter() {
        let on_world = matches!(&rule.when, When::Change(watch) if watches_world(canon, watch));
        let (message, hint) = if on_world && needs_subject(rule) {
            (
                "watches a world change, which has no self and no here",
                "name the actor, or world",
            )
        } else if !on_world && narrates_to_present(rule) {
            (
                "has a narrate with no to",
                "add to: self, here, or an actor id",
            )
        } else {
            continue;
        };
        diagnostics.push(
            schema_error(
                &rule.file,
                &format!("rule '{}' {message}", rule.local_id),
                hint,
            )
            .at_path(format!("rules[{}]", rule.index)),
        );
    }
    diagnostics
}

fn narrates_to_present(rule: &Rule) -> bool {
    let mut present = false;
    rule.visit_templates(|effect| {
        present |= matches!(
            effect,
            EffectTemplate::Narrate {
                to: NarrateTo::Present,
                ..
            }
        );
    });
    present
}

/// Whether `rule` needs a subject: `self` or `here` named as `of`, `subject` or `to`, a
/// nested check's `by`, a narrate's `to`, a location effect, which moves the subject, or a
/// check, which is the subject's.
fn needs_subject(rule: &Rule) -> bool {
    let mut needs = rule.check.is_some();
    rule.visit_templates(|effect| match effect {
        EffectTemplate::Narrate { to, .. } => {
            needs |= matches!(to, NarrateTo::Self_ | NarrateTo::Here);
        }
        EffectTemplate::Location { .. } => needs = true,
        _ => {}
    });
    let self_or_here = [
        grammar::Object::Actor(grammar::Actor::Self_),
        grammar::Object::Here,
    ];
    let owners = rule.mentions().owners;
    needs || self_or_here.iter().any(|of| owners.contains(of))
}

fn mark_cycles(next: &[Vec<usize>], in_cycle: &mut [bool]) {
    let mut color = vec![0u8; next.len()];
    let mut stack = Vec::new();
    for start in 0..next.len() {
        if color[start] == 0 {
            walk_cycle(start, next, &mut color, &mut stack, in_cycle);
        }
    }
}

fn walk_cycle(
    node: usize,
    next: &[Vec<usize>],
    color: &mut [u8],
    stack: &mut Vec<usize>,
    in_cycle: &mut [bool],
) {
    color[node] = 1;
    stack.push(node);
    for &to in &next[node] {
        if color[to] == 1 {
            if let Some(at) = stack.iter().position(|item| *item == to) {
                for item in &stack[at..] {
                    in_cycle[*item] = true;
                }
            }
        } else if color[to] == 0 {
            walk_cycle(to, next, color, stack, in_cycle);
        }
    }
    stack.pop();
    color[node] = 2;
}
