//! Change-rule cycle and reach checks.

use crate::canon::Canon;
use crate::diagnostic::Diagnostic;
use crate::error::Reason;

/// Change rules that set each other's watched values.
pub fn trigger_cycles(canon: &Canon) -> Vec<Diagnostic> {
    let rules: Vec<&crate::rule::Rule> = canon
        .rules
        .iter()
        .filter(|rule| matches!(rule.when, crate::rule::When::Change(_)))
        .collect();
    let written: Vec<Vec<Mark>> = rules
        .iter()
        .map(|rule| {
            let mut marks = Vec::new();
            each_effect(rule, |effect| marks_of(effect, &mut marks));
            marks
        })
        .collect();
    let mut next = vec![Vec::new(); rules.len()];
    for (from, marks) in written.iter().enumerate() {
        for (to, rule) in rules.iter().enumerate() {
            if from == to {
                continue;
            }
            let crate::rule::When::Change(watch) = &rule.when else {
                continue;
            };
            if marks.iter().any(|mark| sets_watch(watch, rule, mark)) {
                next[from].push(to);
            }
        }
    }
    let mut hit = vec![false; rules.len()];
    mark_cycles(&next, &mut hit);
    rules
        .iter()
        .enumerate()
        .filter(|(index, _)| hit[*index])
        .map(|(_, rule)| {
            Diagnostic::new(
                &rule.file,
                "WARNING",
                &format!("rule '{}' is in a trigger cycle", rule.local_id),
                "stop one rule in the cycle from setting the next watched value",
            )
            .with_path(format!("rules[{}]", rule.index))
        })
        .collect()
}

/// Narrate and note on a change rule that can reach a viewer who cannot see the change.
pub fn trigger_leaks(canon: &Canon, now: Option<&crate::model::Now>) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    for rule in canon.rules.iter() {
        let crate::rule::When::Change(watch) = &rule.when else {
            continue;
        };
        let sight = sight_of(canon, now, watch);
        let mut leak = false;
        each_effect(rule, |effect| {
            if reach_of(effect).is_some_and(|reach| too_wide(&sight, &reach)) {
                leak = true;
            }
        });
        if !leak {
            continue;
        }
        diags.push(
            Diagnostic::new(
                &rule.file,
                "TRIGGER_LEAK",
                &format!(
                    "rule '{}' leaks a change to a viewer who cannot see it",
                    rule.local_id
                ),
                "address that output to self, or to here when the room can see the change",
            )
            .with_path(format!("rules[{}]", rule.index)),
        );
    }
    diags
}

#[derive(Clone)]
enum Mark {
    Flag { key: String, value: bool },
    Meter(String),
    Item { id: String, gained: bool },
    Clock(String),
    Arrive(String),
    Knows(String),
}

enum Reach {
    Subject,
    Room,
    Named(Vec<String>),
    Wider,
}

enum Sight {
    Subject,
    Room,
    Owners(Vec<String>),
    Everyone,
}

fn marks_of(effect: &crate::rule::EffectTemplate, out: &mut Vec<Mark>) {
    use crate::rule::{EffectTemplate, LocationTo};
    match effect {
        EffectTemplate::Flag { key, value, .. } => {
            out.push(Mark::Flag {
                key: key.clone(),
                value: *value,
            });
        }
        EffectTemplate::Meter { key, .. } => out.push(Mark::Meter(key.clone())),
        EffectTemplate::Item { id, add, .. } => {
            if *add > 0 {
                out.push(Mark::Item {
                    id: id.clone(),
                    gained: true,
                });
            } else if *add < 0 {
                out.push(Mark::Item {
                    id: id.clone(),
                    gained: false,
                });
            }
        }
        EffectTemplate::Clock { id, add, .. } if add.is_some() => {
            out.push(Mark::Clock(id.clone()));
        }
        EffectTemplate::Location {
            to: LocationTo::Place(id),
            ..
        } => out.push(Mark::Arrive(id.clone())),
        EffectTemplate::Reveal { fact, .. } => out.push(Mark::Knows(fact.clone())),
        EffectTemplate::Memory { fact_ids, .. } => {
            for fact in fact_ids {
                out.push(Mark::Knows(fact.clone()));
            }
        }
        _ => {}
    }
}

fn sets_watch(watch: &crate::rule::ChangeWatch, rule: &crate::rule::Rule, mark: &Mark) -> bool {
    use crate::rule::ChangeWatch;
    match (watch, mark) {
        (
            ChangeWatch::Flag { key, becomes },
            Mark::Flag {
                key: written,
                value,
            },
        ) => key == written && becomes == value,
        (ChangeWatch::Meter { key, .. }, Mark::Meter(written)) => key == written,
        (
            ChangeWatch::Item { id, gained },
            Mark::Item {
                id: written,
                gained: up,
            },
        ) => id == written && gained == up,
        (ChangeWatch::Clock { id }, Mark::Clock(written)) => id == written,
        (ChangeWatch::Arrive { place, .. }, Mark::Arrive(dest)) => {
            rule.watched_place(place) == dest
        }
        (ChangeWatch::Knows { fact }, Mark::Knows(written)) => fact == written,
        _ => false,
    }
}

fn sight_of(
    canon: &Canon,
    now: Option<&crate::model::Now>,
    watch: &crate::rule::ChangeWatch,
) -> Sight {
    use crate::rule::ChangeWatch;
    match watch {
        ChangeWatch::Knows { .. } => Sight::Subject,
        ChangeWatch::Clock { id } => Sight::Owners(
            now.and_then(|now| now.clocks.get(id))
                .map(|clock| clock.owners.clone())
                .unwrap_or_default(),
        ),
        ChangeWatch::Flag { key, .. } if flag_is_public(canon, key) => Sight::Everyone,
        ChangeWatch::Arrive { .. }
        | ChangeWatch::Leave { .. }
        | ChangeWatch::Meter { .. }
        | ChangeWatch::Flag { .. }
        | ChangeWatch::Item { .. } => Sight::Room,
    }
}

fn flag_is_public(canon: &Canon, key: &str) -> bool {
    use crate::rule::{EffectTemplate, Who};
    let mut world = false;
    let mut actor = false;
    for rule in canon.rules.iter() {
        each_effect(rule, |effect| {
            if let EffectTemplate::Flag { who, key: flag, .. } = effect {
                if flag == key {
                    if matches!(who, Who::World) {
                        world = true;
                    } else {
                        actor = true;
                    }
                }
            }
        });
    }
    world && !actor
}

fn reach_of(effect: &crate::rule::EffectTemplate) -> Option<Reach> {
    use crate::rule::{EffectTemplate, NarrateTo};
    match effect {
        EffectTemplate::Narrate { to, .. } => Some(match to {
            NarrateTo::Self_ => Reach::Subject,
            NarrateTo::Here => Reach::Room,
            NarrateTo::Actors(ids) => Reach::Named(ids.clone()),
        }),
        EffectTemplate::Note { to, place, .. } => {
            if place.is_some() {
                Some(Reach::Wider)
            } else {
                Some(Reach::Named(to.clone()))
            }
        }
        _ => None,
    }
}

fn too_wide(sight: &Sight, reach: &Reach) -> bool {
    match sight {
        Sight::Everyone => false,
        Sight::Subject => !matches!(reach, Reach::Subject),
        Sight::Room => !matches!(reach, Reach::Subject | Reach::Room),
        Sight::Owners(owners) => match reach {
            Reach::Subject => false,
            Reach::Named(ids) => ids.iter().any(|id| !owners.iter().any(|owner| owner == id)),
            Reach::Room | Reach::Wider => true,
        },
    }
}

fn mark_cycles(next: &[Vec<usize>], hit: &mut [bool]) {
    let mut color = vec![0u8; next.len()];
    let mut stack = Vec::new();
    for start in 0..next.len() {
        if color[start] == 0 {
            walk_cycle(start, next, &mut color, &mut stack, hit);
        }
    }
}

fn walk_cycle(
    node: usize,
    next: &[Vec<usize>],
    color: &mut [u8],
    stack: &mut Vec<usize>,
    hit: &mut [bool],
) {
    color[node] = 1;
    stack.push(node);
    for &to in &next[node] {
        if color[to] == 1 {
            if let Some(at) = stack.iter().position(|item| *item == to) {
                for item in &stack[at..] {
                    hit[*item] = true;
                }
            }
        } else if color[to] == 0 {
            walk_cycle(to, next, color, stack, hit);
        }
    }
    stack.pop();
    color[node] = 2;
}

fn each_effect(rule: &crate::rule::Rule, mut visit: impl FnMut(&crate::rule::EffectTemplate)) {
    walk(&rule.do_, &mut visit);
    walk(&rule.success, &mut visit);
    walk(&rule.partial, &mut visit);
    walk(&rule.failure, &mut visit);
}

fn walk(
    effects: &[crate::rule::EffectTemplate],
    visit: &mut impl FnMut(&crate::rule::EffectTemplate),
) {
    use crate::rule::ResistSpec;
    for effect in effects {
        visit(effect);
        if let Some(ResistSpec::Roll { resisted, .. }) = effect.resist() {
            walk(resisted, visit);
        }
    }
}

/// Markdown with the body removed. Frontmatter stays. The trailing delimiter is kept.
pub fn strip_body(text: &str) -> Result<String, Reason> {
    let rest = text.strip_prefix("---\n").ok_or(Reason::Schema)?;
    let index = rest.find("\n---\n").ok_or(Reason::Schema)?;
    Ok(format!("---\n{}\n---\n", &rest[..index]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_body_rejects_a_missing_fence() {
        assert_eq!(strip_body("no fence\n"), Err(Reason::Schema));
        assert_eq!(strip_body("---\nno closer\n"), Err(Reason::Schema));
    }
}
