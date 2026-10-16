use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::canon::{load_canon, Canon};
use crate::diagnostic::{locate_nth, reason_of, schema, Diagnostic};
use crate::error::Reason;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LintHit {
    pub path: String,
    pub text: String,
}

/// Hidden fact texts found as case-sensitive substrings of a Markdown body.
/// An empty vec is a clean pack.
pub fn lint(root: &Path) -> Result<Vec<LintHit>, Reason> {
    let canon = load_canon(root).map_err(reason_of)?;
    let (leaks, problems) = scan(root, &canon);
    if !problems.is_empty() {
        return Err(reason_of(problems));
    }
    Ok(leaks
        .into_iter()
        .map(|leak| LintHit {
            path: leak.path,
            text: leak.text,
        })
        .collect())
}

/// Hidden-fact sentences found in Markdown bodies, plus files the scan could not read.
pub(crate) fn body_leaks(root: &Path, canon: &Canon) -> Vec<Diagnostic> {
    let (leaks, mut problems) = scan(root, canon);
    for leak in leaks {
        let mut diag = Diagnostic::new(
            &leak.path,
            "HIDDEN_FACT_IN_BODY",
            &format!("hidden fact '{}' is in the body", leak.text),
            "remove that sentence from the prose, or set hidden: false",
        );
        if let Some(line) = leak.line {
            diag = diag.at_line(line, None);
        }
        problems.push(diag);
    }
    problems
}

/// Hidden-fact sentences found in a message template's `text`.
/// `HIDDEN_FACT_IN_BODY` stays the Markdown scan. A move row is not a body.
pub(crate) fn message_text_leaks(root: &Path, canon: &Canon) -> Vec<Diagnostic> {
    let needles = hidden_needles(canon);
    if needles.is_empty() {
        return Vec::new();
    }
    let mut hits = Vec::new();
    for rule in canon.rules.iter() {
        for effect in rule
            .do_
            .iter()
            .chain(rule.success.iter())
            .chain(rule.partial.iter())
            .chain(rule.failure.iter())
        {
            let Some(text) = effect.note_text() else {
                continue;
            };
            if needles.iter().any(|needle| text.contains(needle)) {
                hits.push((rule.file.clone(), text.to_string()));
            }
        }
    }
    hits.sort();
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    let mut problems = Vec::new();
    for (rel, text) in hits {
        let Ok(file) = fs::read_to_string(root.join(&rel)) else {
            continue;
        };
        let nth = seen.entry(format!("{rel}:{text}")).or_insert(0);
        let spot = locate_nth(&file, &text, *nth);
        *nth += 1;
        for needle in &needles {
            if !text.contains(needle) {
                continue;
            }
            let mut diag = Diagnostic::new(
                &rel,
                "HIDDEN_FACT_IN_TEXT",
                &format!("hidden fact '{needle}' is in a note"),
                "rewrite that note, or set hidden: false",
            );
            if let Some((line, column)) = spot {
                diag = diag.at_line(line, Some(column));
            }
            problems.push(diag);
        }
    }
    problems
}

fn hidden_needles(canon: &Canon) -> Vec<String> {
    let mut needles: Vec<String> = canon
        .facts
        .values()
        .filter(|fact| fact.hidden && !fact.text.is_empty())
        .map(|fact| fact.text.clone())
        .collect();
    needles.sort();
    needles.dedup();
    needles
}

struct Leak {
    path: String,
    line: Option<u32>,
    text: String,
}

fn scan(root: &Path, canon: &Canon) -> (Vec<Leak>, Vec<Diagnostic>) {
    let mut needles: Vec<String> = canon
        .facts
        .values()
        .filter(|fact| fact.hidden)
        .map(|fact| fact.text.clone())
        .collect();
    needles.sort();
    needles.dedup();

    let mut files = Vec::new();
    let mut problems = Vec::new();
    if let Err(reason) = collect_markdown(root, root, &mut files) {
        problems.push(
            schema(
                "pack",
                "a Markdown file could not be read",
                "check that cast/ and world/ are readable UTF-8",
            )
            .with_path(reason.token()),
        );
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));

    let mut leaks = Vec::new();
    for (rel, text) in files {
        let body = match markdown_body(&text) {
            Ok(body) => body,
            Err(_) => {
                problems.push(
                    schema(
                        &rel,
                        "this Markdown file has no frontmatter fence",
                        "wrap the header in --- lines",
                    )
                    .at_line(1, Some(1)),
                );
                continue;
            }
        };
        let body_at = text.find(body).unwrap_or(0);
        let base = text[..body_at].chars().filter(|ch| *ch == '\n').count() as u32;
        for needle in &needles {
            if !body.contains(needle) {
                continue;
            }
            let line = body
                .lines()
                .position(|line| line.contains(needle))
                .map(|index| base + index as u32 + 1);
            leaks.push(Leak {
                path: rel.clone(),
                line,
                text: needle.clone(),
            });
        }
    }
    (leaks, problems)
}

fn collect_markdown(
    dir: &Path,
    root: &Path,
    out: &mut Vec<(String, String)>,
) -> Result<(), Reason> {
    if !dir.exists() {
        return Ok(());
    }
    let mut entries = fs::read_dir(dir)
        .map_err(|_| Reason::Schema)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| Reason::Schema)?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect_markdown(&path, root, out)?;
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("md") {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .map_err(|_| Reason::Schema)?
            .components()
            .map(|component| component.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        let text = fs::read_to_string(&path).map_err(|_| Reason::Schema)?;
        out.push((rel, text));
    }
    Ok(())
}

/// Change rules that set each other's watched values.
pub fn trigger_cycles(canon: &Canon) -> Vec<Diagnostic> {
    let rules: Vec<&crate::rules::rule::Rule> = canon
        .rules
        .iter()
        .filter(|rule| matches!(rule.when, crate::rules::rule::When::Change(_)))
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
            let crate::rules::rule::When::Change(watch) = &rule.when else {
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
        let crate::rules::rule::When::Change(watch) = &rule.when else {
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

fn marks_of(effect: &crate::rules::rule::EffectTemplate, out: &mut Vec<Mark>) {
    use crate::rules::rule::{EffectTemplate, LocationTo};
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

fn sets_watch(
    watch: &crate::rules::rule::ChangeWatch,
    rule: &crate::rules::rule::Rule,
    mark: &Mark,
) -> bool {
    use crate::rules::rule::ChangeWatch;
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
            watched_place(rule, place) == dest
        }
        (ChangeWatch::Knows { fact }, Mark::Knows(written)) => fact == written,
        _ => false,
    }
}

fn watched_place<'a>(rule: &'a crate::rules::rule::Rule, place: &'a str) -> &'a str {
    if place == "here" {
        if let crate::rules::rule::Owner::Place(id) = &rule.owner {
            return id;
        }
    }
    place
}

fn sight_of(
    canon: &Canon,
    now: Option<&crate::model::Now>,
    watch: &crate::rules::rule::ChangeWatch,
) -> Sight {
    use crate::rules::rule::ChangeWatch;
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
    use crate::rules::rule::{EffectTemplate, Who};
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

fn reach_of(effect: &crate::rules::rule::EffectTemplate) -> Option<Reach> {
    use crate::rules::rule::{EffectTemplate, NarrateTo};
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

fn each_effect(
    rule: &crate::rules::rule::Rule,
    mut visit: impl FnMut(&crate::rules::rule::EffectTemplate),
) {
    walk(&rule.do_, &mut visit);
    walk(&rule.success, &mut visit);
    walk(&rule.partial, &mut visit);
    walk(&rule.failure, &mut visit);
}

fn walk(
    effects: &[crate::rules::rule::EffectTemplate],
    visit: &mut impl FnMut(&crate::rules::rule::EffectTemplate),
) {
    use crate::rules::rule::ResistSpec;
    for effect in effects {
        visit(effect);
        if let Some(ResistSpec::Roll { resisted, .. }) = effect.resist() {
            walk(resisted, visit);
        }
    }
}

fn markdown_body(text: &str) -> Result<&str, Reason> {
    let rest = text.strip_prefix("---\n").ok_or(Reason::Schema)?;
    let index = rest.find("\n---\n").ok_or(Reason::Schema)?;
    Ok(&rest[index + 5..])
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

    #[test]
    fn markdown_body_rejects_a_missing_fence() {
        assert_eq!(markdown_body("no fence\n"), Err(Reason::Schema));
        assert_eq!(markdown_body("---\nno closer\n"), Err(Reason::Schema));
    }
}
