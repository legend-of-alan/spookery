//! Cross-file validation of rules, paths, meters, items, and triggers.

mod entity_scope;
mod trigger;

use std::collections::{BTreeMap, BTreeSet};

use crate::canon::map::MAP_FILE;
use crate::canon::{Canon, ItemDef, PathDef, PlaceDef, OPEN};
use crate::diagnostic::{schema_error, warning, Diagnostic};
use crate::ecs::component::modifier;
use crate::ecs::id::PlaceId;
use crate::rules::cond;
use crate::rules::grammar;
use crate::rules::rule::{self, ChangeWatch, Cond, EffectTemplate, Owner, When};

pub(super) use entity_scope::{check_carriers, check_meter_reads, EntityScope};

/// Paths whose ends are not places, and items a path names but the pack does not.
pub(super) fn check_paths(
    paths: &[PathDef],
    places: &BTreeMap<PlaceId, PlaceDef>,
    items: &BTreeMap<String, ItemDef>,
) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    for (row, path) in paths.iter().enumerate() {
        let arrow = if path.bidirectional { "<->" } else { "->" };
        let line = format!(
            "{} {arrow} {}",
            path.end_place_ids[0], path.end_place_ids[1]
        );
        let mut refuse = |field: &str, problem: String, hint: &str| {
            let at = format!("paths[{row}]{field}");
            diags.push(schema_error(MAP_FILE, &problem, hint).at_path(at));
        };
        for end_place_id in (path.end_place_ids.iter()).filter(|id| !places.contains_key(*id)) {
            let problem = format!("path '{line}' names '{end_place_id}', which is not a place");
            refuse(".path", problem, "name a place from world/places");
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

pub(super) fn check_rule_items(
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
pub(super) fn check_modifiers(
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

/// Static warnings and trigger-reach errors on a loaded `Canon`, whose save lists the flag keys in
/// `held_flags`.
pub(crate) fn check_canon(canon: &Canon, held_flags: &BTreeSet<&str>) -> Vec<Diagnostic> {
    let mut diags = check_reachable_places(canon);
    diags.extend(check_teleport_verbs(canon));
    diags.extend(trigger::check_cycles(canon));
    diags.extend(trigger::check_leaks(canon));
    diags.extend(trigger::check_world_change_subjects(canon));
    diags.extend(check_cycle_conditions(canon));
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

/// Flag conditions, on a rule or a path, a change rule's flag watch and a path's `revealed_by`,
/// that name a key no rule sets and `held` does not list, so its value never moves: most likely
/// a misspelt key. `open` gets no warning here: the round sets it on paths, not through rules,
/// and `check_carriers` checks it in a condition.
fn check_unset_flags(canon: &Canon, held: &BTreeSet<&str>) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    let is_ever_set = |key| held.contains(key) || find_flag_holders(canon, key).next().is_some();
    let hint = "set the flag in a rule or the save, or name a flag that is set";
    let unset_problem =
        |key: &str| format!("names flag '{key}', which no rule sets and no save holds");
    for (file, at, conds) in conditions(canon) {
        for cond in conds {
            cond.visit(|cond| {
                let Cond::Flag { key, .. } = cond else {
                    return;
                };
                if key == OPEN || is_ever_set(key.as_str()) {
                    return;
                }
                diags.push(warning(file, &unset_problem(key), hint).at_path(at.clone()));
            });
        }
    }
    for rule in canon.rules.iter() {
        let When::Change(ChangeWatch::Flag { key, .. }) = &rule.when else {
            continue;
        };
        if key != OPEN && !is_ever_set(key) {
            let at = format!("rules[{}].when", rule.index);
            diags.push(warning(&rule.file, &unset_problem(key), hint).at_path(at));
        }
    }
    for (row, path) in canon.paths.iter().enumerate() {
        let Some(key) = path.revealed_by.as_deref().filter(|key| !is_ever_set(key)) else {
            continue;
        };
        let at = format!("paths[{row}].revealed_by");
        diags.push(warning(MAP_FILE, &unset_problem(key), hint).at_path(at));
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
fn check_cycle_conditions(canon: &Canon) -> Vec<Diagnostic> {
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
        let exits = canon.exits(&place);
        queue.extend(exits.map(|exit| exit.to_place_id().to_string()));
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

/// The entities a rule can make hold flag `key`: the `of` of each flag effect that writes it.
fn find_flag_holders<'c>(canon: &'c Canon, key: &str) -> impl Iterator<Item = &'c grammar::Object> {
    let mut holders = Vec::new();
    for rule in canon.rules.iter() {
        rule.visit_templates(|effect| match effect {
            EffectTemplate::Flag {
                of, key: written, ..
            } if written == key => holders.push(of),
            _ => {}
        });
    }
    holders.into_iter()
}
