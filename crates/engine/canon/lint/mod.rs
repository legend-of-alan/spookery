//! Cross-file validation of rules, paths, meters, items, facts, and triggers.

mod entity_scope;
mod trigger;

use std::collections::{BTreeMap, BTreeSet};

use crate::canon::map::MAP_FILE;
use crate::canon::{Canon, FactDef, ItemDef, PathDef, PlaceDef};
use crate::diagnostic::{schema_error, warning, Diagnostic};
use crate::ecs::component::modifier;
use crate::ecs::id::PlaceId;
use crate::rules::cond;
use crate::rules::grammar;
use crate::rules::rule::{self, Cond, EffectTemplate, Owner};

pub(super) use entity_scope::{unknown_carriers, unknown_reads, EntityScope};
use trigger::{check_trigger_cycles, check_trigger_leaks, check_world_change_subjects};

/// Paths whose ends are not places, and facts or items a path names but the pack does not.
pub(super) fn broken_paths(
    paths: &[PathDef],
    places: &BTreeMap<PlaceId, PlaceDef>,
    facts: &BTreeMap<String, FactDef>,
    items: &BTreeMap<String, ItemDef>,
) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    for (row, path) in paths.iter().enumerate() {
        let arrow = if path.bidirectional { "<->" } else { "->" };
        let line = format!("{} {arrow} {}", path.ends[0], path.ends[1]);
        let mut refuse = |field: &str, problem: String, hint: &str| {
            let at = format!("paths[{row}]{field}");
            diags.push(schema_error(MAP_FILE, &problem, hint).at_path(at));
        };
        for end in path.ends.iter().filter(|end| !places.contains_key(*end)) {
            let problem = format!("path '{line}' names '{end}', which is not a place");
            refuse(".path", problem, "name a place from world/places");
        }
        if path.hidden && path.revealed_by.is_none() {
            let problem = format!("hidden path '{line}' has no revealed_by fact");
            refuse(".revealed_by", problem, "set revealed_by to a fact id");
        }
        if let Some(fact) = (path.revealed_by.as_ref()).filter(|fact| !facts.contains_key(*fact)) {
            let problem = format!("path '{line}' reveals '{fact}', which is not a fact");
            refuse(".revealed_by", problem, "use a fact id from this pack");
        }
        for id in path.mentions().item_ids {
            if !items.contains_key(id) {
                let problem = format!("path '{line}' names unknown item '{id}'");
                refuse("", problem, &format!("add world/items/{id}.md"));
            }
        }
    }
    diags
}

pub(super) fn unknown_rule_items(
    rules: &rule::Table,
    items: &BTreeMap<String, ItemDef>,
) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    for rule in rules.iter() {
        for id in rule.mentions().item_ids {
            if items.contains_key(id) {
                continue;
            }
            diags.push(unknown_item(
                &rule.file,
                id,
                &format!("rules[{}]", rule.index),
            ));
        }
    }
    diags
}

/// Modifier effects naming a modifier `rules/modifiers.yaml` does not define.
pub(super) fn unknown_modifiers(
    rules: &rule::Table,
    modifiers: &BTreeMap<String, modifier::Descriptor>,
) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    for rule in rules.iter() {
        rule.visit_templates(|effect| match effect {
            EffectTemplate::Modifier { id, .. } if !modifiers.contains_key(id) => {
                diags.push(
                    schema_error(
                        &rule.file,
                        &format!(
                            "rule '{}' applies modifier '{id}', which is not in rules/modifiers.yaml",
                            rule.local_id
                        ),
                        "add the modifier or name one that exists",
                    )
                    .at_path(format!("rules[{}]", rule.index)),
                );
            }
            _ => {}
        });
    }
    diags
}

pub(crate) fn unknown_item(file: &str, id: &str, path: &str) -> Diagnostic {
    schema_error(
        file,
        &format!("unknown item '{id}'"),
        &format!("add world/items/{id}.md"),
    )
    .at_path(path)
}

pub(super) fn reveal_problems(
    rules: &rule::Table,
    facts: &BTreeMap<String, FactDef>,
) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    for rule in rules.iter() {
        rule.visit_templates(|effect| {
            let EffectTemplate::Reveal { fact, .. } = effect else {
                return;
            };
            if !facts.contains_key(fact) {
                diags.push(
                    schema_error(
                        &rule.file,
                        &format!(
                            "rule '{}' reveals '{fact}', which is not a fact",
                            rule.local_id
                        ),
                        "name a fact from this pack",
                    )
                    .at_path(format!("rules[{}]", rule.index)),
                );
            }
        });
    }
    diags
}

/// Static warnings and trigger-reach errors on a loaded `Canon`, whose save lists the flag keys in
/// `held_flags`.
pub(crate) fn check_canon(canon: &Canon, held_flags: &BTreeSet<&str>) -> Vec<Diagnostic> {
    let mut diags = check_reachable_places(canon);
    diags.extend(check_teleport_verbs(canon));
    diags.extend(check_trigger_cycles(canon));
    diags.extend(check_trigger_leaks(canon));
    diags.extend(check_world_change_subjects(canon));
    diags.extend(check_cycles(canon));
    diags.extend(check_gm_notes_length(canon));
    diags.extend(check_unset_flags(canon, held_flags));
    diags
}

/// Each rule's and each path's conditions, with the file and path that hold them.
fn conditions(canon: &Canon) -> impl Iterator<Item = (&str, String, &[Cond])> {
    let rules = (canon.rules.iter()).map(|rule| {
        (
            rule.file.as_str(),
            format!("rules[{}].if", rule.index),
            &rule.if_[..],
        )
    });
    let paths = (canon.paths.iter().enumerate())
        .map(|(row, path)| (MAP_FILE, format!("paths[{row}].if"), &path.if_[..]));
    rules.chain(paths)
}

/// Flag conditions, on a rule or a path, that test a key no rule sets and `held` does not list,
/// so its value never moves: most likely a misspelt key. An exit's `open` is left to
/// `unknown_carriers`.
fn check_unset_flags(canon: &Canon, held: &BTreeSet<&str>) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    for (file, at, conds) in conditions(canon) {
        for cond in conds {
            cond.visit(|cond| {
                let Cond::Flag { of, key, .. } = cond else {
                    return;
                };
                let is_ever_set =
                    |key| held.contains(key) || flag_owners(canon, key).next().is_some();
                if matches!(of, grammar::Object::Exit(_)) || is_ever_set(key.as_str()) {
                    return;
                }
                let problem = format!("tests flag '{key}', which no rule sets and no save holds");
                let hint = "set the flag in a rule or the save, or test a flag that is set";
                diags.push(warning(file, &problem, hint).at_path(at.clone()));
            });
        }
    }
    diags
}

/// The longest `gm_notes`, in characters (about 250 tokens), that `spook check` passes without a
/// warning. A local model under Ollama may have only a few thousand tokens of context, shared on
/// every call by the AI GM's instructions, its tools, the game state and the notes.
const MAX_GM_NOTES_CHARS: usize = 1000;

/// `gm_notes` past `MAX_GM_NOTES_CHARS` is a warning, so an author can still go over on purpose.
fn check_gm_notes_length(canon: &Canon) -> Option<Diagnostic> {
    let chars = canon.gm_notes.chars().count();
    (chars > MAX_GM_NOTES_CHARS).then(|| {
        warning(
            "pack.yaml",
            &format!(
                "gm_notes is {chars} characters, over the cap of {MAX_GM_NOTES_CHARS} characters"
            ),
            "cut it to the pack's tone, narrative style and story goal",
        )
        .at_path("gm_notes")
    })
}

/// `cycle` conditions, on a rule or a path, that name no cycle in the budget or have a bound
/// at or past the cycle's length.
fn check_cycles(canon: &Canon) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    for (file, at, conds) in conditions(canon) {
        let mut cycles = Vec::new();
        for cond in conds {
            cond.visit(|cond| {
                if let Cond::Cycle { id, from, to } = cond {
                    cycles.push((id.as_str(), [from.constant(), to.constant()]));
                }
            });
        }
        for (id, constant_bounds) in cycles {
            let Some(&length) = canon.budget.cycles.get(id) else {
                diags.push(
                    schema_error(
                        file,
                        &format!("there is no cycle {id}"),
                        "name a cycle from cycles in rules/budget.yaml, or add it there",
                    )
                    .at_path(at.clone()),
                );
                continue;
            };
            let is_outside = |bound| cond::cycle_tick_of(bound, length).is_none();
            if constant_bounds.into_iter().flatten().any(is_outside) {
                diags.push(
                    schema_error(
                        file,
                        &format!("cycle bound is not below cycle {id}, which is {length} ticks"),
                        "set from and to to ticks of the cycle, below its length",
                    )
                    .at_path(at.clone()),
                );
            }
        }
    }
    diags
}

/// Places no walk reaches from the start, including through hidden and locked exits.
fn check_reachable_places(canon: &Canon) -> Vec<Diagnostic> {
    let mut seen = BTreeSet::new();
    let mut queue = vec![canon.start_place.clone()];
    while let Some(place) = queue.pop() {
        if !seen.insert(place.clone()) {
            continue;
        }
        queue.extend(canon.exits(&place).map(|exit| exit.to.to_string()));
    }
    let mut diags = Vec::new();
    for id in canon.places.keys() {
        if seen.contains(id.as_str()) {
            continue;
        }
        diags.push(
            warning(
                "world/places",
                &format!("place '{id}' has no path from {}", canon.start_place),
                "add a path in world/map.yaml that reaches it",
            )
            .at_path(id.clone()),
        );
    }
    diags
}

/// `teleport: true` in the shared verbs file is a warning. Author rules may teleport.
fn check_teleport_verbs(canon: &Canon) -> Vec<Diagnostic> {
    canon
        .rules
        .iter()
        .filter(|rule| {
            rule.owner == Owner::Verb
                && rule
                    .templates()
                    .into_iter()
                    .any(|effect| matches!(effect, EffectTemplate::Location { teleport: true, .. }))
        })
        .map(|rule| {
            warning(
                &rule.file,
                &format!("verb '{}' teleports", rule.local_id),
                "teleport belongs on an author rule, not in verbs.yaml",
            )
            .at_path(format!("rules[{}]", rule.index))
        })
        .collect()
}

/// The `of` of each flag effect in the pack's rules that writes flag `key`.
fn flag_owners<'c>(canon: &'c Canon, key: &str) -> impl Iterator<Item = &'c grammar::Object> {
    let mut owners = Vec::new();
    for rule in canon.rules.iter() {
        rule.visit_templates(|effect| match effect {
            EffectTemplate::Flag {
                of, key: written, ..
            } if written == key => owners.push(of),
            _ => {}
        });
    }
    owners.into_iter()
}
