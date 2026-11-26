//! Lints on a pack: cross-file lints on rules, paths, meters, items and triggers, and the
//! per-rule lints that compile runs (`rule`).

pub(super) mod entity_defs;
pub(super) mod rule;
mod trigger;

use std::collections::{BTreeMap, BTreeSet};

use crate::canon::map::{self, PathDef, OPEN};
use crate::canon::{item, place, Canon};
use crate::diagnostic::{schema_error, warning, Diagnostic};
use crate::ecs::component::modifier;
use crate::ecs::id::PlaceId;
use crate::rules::cond;
use crate::rules::grammar;
use crate::rules::rule::{ChangeWatch, Cond, EffectTemplate, RuleKind, Rules, Scope};

/// Paths whose ends are not places, and items a path names but the pack does not.
pub(super) fn find_unknown_path_ids(
    path_defs: &[PathDef],
    places: &BTreeMap<PlaceId, place::Def>,
    items: &BTreeMap<String, item::Def>,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for (index, path_def) in path_defs.iter().enumerate() {
        let arrow = if path_def.bidirectional { "<->" } else { "->" };
        let path_text = format!(
            "{} {arrow} {}",
            path_def.end_place_ids[0], path_def.end_place_ids[1]
        );
        let mut refuse = |field: &str, message: String, hint: &str| {
            let key_path = format!("paths[{index}]{field}");
            diagnostics.push(schema_error(map::FILE, &message, hint).at_key_path(key_path));
        };
        for end_place_id in (path_def.end_place_ids.iter()).filter(|id| !places.contains_key(*id)) {
            let message =
                format!("path '{path_text}' names '{end_place_id}', which is not a place");
            refuse(".path", message, "name a place from world/places");
        }
        for id in path_def.mentions().item_ids {
            if !items.contains_key(id) {
                let message = format!("path '{path_text}' names unknown item '{id}'");
                refuse("", message, &format!("add world/items/{id}.md"));
            }
        }
    }
    diagnostics
}

pub(super) fn find_unknown_rule_items(
    rules: &Rules,
    items: &BTreeMap<String, item::Def>,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for rule in rules.iter() {
        for id in rule.mentions().item_ids {
            if items.contains_key(id) {
                continue;
            }
            diagnostics.push(refuse_unknown_item(
                &rule.file,
                id,
                &format!("rules[{}]", rule.index),
            ));
        }
    }
    diagnostics
}

/// Modifier effects naming a modifier `rules/modifiers.yaml` does not define.
pub(super) fn find_unknown_modifiers(
    rules: &Rules,
    modifiers: &BTreeMap<String, modifier::Def>,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for rule in rules.iter() {
        rule.visit_templates(|effect| match effect {
            EffectTemplate::Modifier { id, .. } if !modifiers.contains_key(id) => {
                diagnostics.push(
                    schema_error(
                        &rule.file,
                        &format!(
                            "rule '{}' applies modifier '{id}', which is not in rules/modifiers.yaml",
                            rule.local_id
                        ),
                        "add the modifier or name one that exists",
                    )
                    .at_key_path(format!("rules[{}]", rule.index)),
                );
            }
            _ => {}
        });
    }
    diagnostics
}

pub(crate) fn refuse_unknown_item(file: &str, id: &str, key_path: &str) -> Diagnostic {
    schema_error(
        file,
        &format!("unknown item '{id}'"),
        &format!("add world/items/{id}.md"),
    )
    .at_key_path(key_path)
}

/// Static warnings and trigger-reach errors on a loaded `Canon`, whose save lists the flag keys in
/// `saved_flags`.
pub(crate) fn run(canon: &Canon, saved_flags: &BTreeSet<&str>) -> Vec<Diagnostic> {
    let mut diagnostics = find_unreachable_places(canon);
    diagnostics.extend(find_teleport_verbs(canon));
    diagnostics.extend(trigger::find_cycles(canon));
    diagnostics.extend(trigger::find_leaks(canon));
    diagnostics.extend(trigger::find_world_change_subjects(canon));
    diagnostics.extend(find_bad_cycle_conds(canon));
    diagnostics.extend(find_long_gm_notes(canon));
    diagnostics.extend(find_unset_flags(canon, saved_flags));
    diagnostics
}

/// Each rule's and each path's conds, with the file and key path that hold them.
fn rule_and_path_conds(canon: &Canon) -> impl Iterator<Item = (&str, String, &[Cond])> {
    let rules = (canon.rules.iter()).map(|rule| {
        (
            rule.file.as_str(),
            format!("rules[{}].if", rule.index),
            &rule.if_[..],
        )
    });
    let paths = (canon.paths.iter().enumerate())
        .map(|(index, path_def)| (map::FILE, format!("paths[{index}].if"), &path_def.if_[..]));
    rules.chain(paths)
}

/// Flag conds, on a rule or a path, a change rule's flag watch and a path's `revealed_by`,
/// that name an id no rule sets and `saved_flags` does not list, so its value never moves: most likely
/// a misspelt id. `open` gets no warning here: the round sets it on paths, not through rules,
/// and `find_missing_components` checks it in a cond.
fn find_unset_flags(canon: &Canon, saved_flags: &BTreeSet<&str>) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let is_ever_set =
        |id| saved_flags.contains(id) || find_flag_objects(canon, id).next().is_some();
    let hint = "set the flag in a rule or the save, or name a flag that is set";
    let unset_message =
        |id: &str| format!("names flag '{id}', which no rule sets and no save holds");
    for (file, key_path, conds) in rule_and_path_conds(canon) {
        for cond in conds {
            cond.visit(|cond| {
                let Cond::Flag { id, .. } = cond else {
                    return;
                };
                if id == OPEN || is_ever_set(id.as_str()) {
                    return;
                }
                diagnostics
                    .push(warning(file, &unset_message(id), hint).at_key_path(key_path.clone()));
            });
        }
    }
    for rule in canon.rules.iter() {
        let RuleKind::Change(ChangeWatch::Flag { id, .. }) = &rule.kind else {
            continue;
        };
        if id != OPEN && !is_ever_set(id) {
            let key_path = format!("rules[{}].when", rule.index);
            diagnostics.push(warning(&rule.file, &unset_message(id), hint).at_key_path(key_path));
        }
    }
    for (index, path_def) in canon.paths.iter().enumerate() {
        let Some(id) = path_def
            .revealed_by
            .as_deref()
            .filter(|id| !is_ever_set(id))
        else {
            continue;
        };
        let key_path = format!("paths[{index}].revealed_by");
        diagnostics.push(warning(map::FILE, &unset_message(id), hint).at_key_path(key_path));
    }
    diagnostics
}

/// The longest `gm_notes`, in characters (about 250 tokens), that `spook check` passes without a
/// warning. A local model under Ollama may have only a few thousand tokens of context, shared on
/// every call by the AI GM's instructions, its tools, the game state and the notes.
const MAX_GM_NOTES_CHARS: usize = 1000;

/// `gm_notes` past `MAX_GM_NOTES_CHARS` is a warning, so an author can still go over on purpose.
fn find_long_gm_notes(canon: &Canon) -> Option<Diagnostic> {
    let chars = canon.gm_notes.chars().count();
    (chars > MAX_GM_NOTES_CHARS).then(|| {
        warning(
            "pack.yaml",
            &format!(
                "gm_notes is {chars} characters, over the cap of {MAX_GM_NOTES_CHARS} characters"
            ),
            "cut it to the pack's tone, narrative style and story goal",
        )
        .at_key_path("gm_notes")
    })
}

/// `cycle` conds, on a rule or a path, that name no cycle in the budget or have a bound
/// at or past the cycle's length.
fn find_bad_cycle_conds(canon: &Canon) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for (file, key_path, conds) in rule_and_path_conds(canon) {
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
                diagnostics.push(
                    schema_error(
                        file,
                        &format!("there is no cycle {id}"),
                        "name a cycle from cycles in rules/budget.yaml, or add it there",
                    )
                    .at_key_path(key_path.clone()),
                );
                continue;
            };
            let is_outside = |bound| cond::cycle_tick_of(bound, length).is_none();
            if constant_bounds.into_iter().flatten().any(is_outside) {
                diagnostics.push(
                    schema_error(
                        file,
                        &format!("cycle bound is not below cycle {id}, which is {length} ticks"),
                        "set from and to to ticks of the cycle, below its length",
                    )
                    .at_key_path(key_path.clone()),
                );
            }
        }
    }
    diagnostics
}

/// Places no walk reaches from the start, including through hidden and locked exits.
fn find_unreachable_places(canon: &Canon) -> Vec<Diagnostic> {
    let mut seen = BTreeSet::new();
    let mut queue = vec![canon.start_place.clone()];
    while let Some(place_id) = queue.pop() {
        if !seen.insert(place_id.clone()) {
            continue;
        }
        let exits = canon.paths.exits(&place_id);
        queue.extend(exits.map(|exit| exit.to_place_id().to_string()));
    }
    let mut diagnostics = Vec::new();
    for id in canon.places.keys() {
        if seen.contains(id.as_str()) {
            continue;
        }
        diagnostics.push(
            warning(
                "world/places",
                &format!("place '{id}' has no path from {}", canon.start_place),
                "add a path in world/map.yaml that reaches it",
            )
            .at_key_path(id.clone()),
        );
    }
    diagnostics
}

/// `teleport: true` in the shared verbs file is a warning. Author rules may teleport.
fn find_teleport_verbs(canon: &Canon) -> Vec<Diagnostic> {
    canon
        .rules
        .iter()
        .filter(|rule| {
            rule.scope == Scope::Verb
                && rule
                    .templates()
                    .into_iter()
                    .any(|effect| matches!(effect, EffectTemplate::Move { teleport: true, .. }))
        })
        .map(|rule| {
            warning(
                &rule.file,
                &format!("verb '{}' teleports", rule.local_id),
                "teleport belongs on an author rule, not in verbs.yaml",
            )
            .at_key_path(format!("rules[{}]", rule.index))
        })
        .collect()
}

/// The entities a rule can make hold flag `id`: the `of` of each flag effect that writes it.
fn find_flag_objects<'c>(canon: &'c Canon, id: &str) -> impl Iterator<Item = &'c grammar::Object> {
    let mut objects = Vec::new();
    for rule in canon.rules.iter() {
        rule.visit_templates(|effect| match effect {
            EffectTemplate::Flag {
                of, id: written, ..
            } if written == id => objects.push(of),
            _ => {}
        });
    }
    objects.into_iter()
}
