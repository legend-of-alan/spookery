//! Cross-file validation of rules, exits, checks, meters, items, and facts.

use std::collections::BTreeMap;

use crate::canon::save::unknown_item;
use crate::canon::{CastCanon, FactCanon, ItemCanon, PlaceCanon};
use crate::check::CheckDef;
use crate::diagnostic::{schema as schema_error, Diagnostic};
use crate::meter::MeterDef;
use crate::rule::{Cond, Cost, EffectTemplate, Expr, ResistSpec, Rule, RuleTable, Who};

/// Exits whose `to` is not a place, and facts or items an exit names but the pack does not.
pub(super) fn broken_exits(
    places: &BTreeMap<String, PlaceCanon>,
    facts: &BTreeMap<String, FactCanon>,
    items: &BTreeMap<String, ItemCanon>,
) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    for place in places.values() {
        for exit in &place.exits {
            if !places.contains_key(&exit.to) {
                diags.push(
                    schema_error(
                        &exit.file,
                        &format!(
                            "exit '{}' goes to '{}', which is not a place",
                            exit.name, exit.to
                        ),
                        "set to: to a place id",
                    )
                    .with_path(format!("exits[{}].to", exit.index)),
                );
            }
            if exit.hidden && exit.revealed_by.is_none() {
                diags.push(
                    schema_error(
                        &exit.file,
                        &format!("hidden exit '{}' has no revealed_by fact", exit.name),
                        "set revealed_by to a fact id",
                    )
                    .with_path(format!("exits[{}].revealed_by", exit.index)),
                );
            }
            if let Some(fact) = &exit.revealed_by {
                if !facts.contains_key(fact) {
                    diags.push(
                        schema_error(
                            &exit.file,
                            &format!("exit '{}' reveals '{fact}', which is not a fact", exit.name),
                            "use a fact id from this pack",
                        )
                        .with_path(format!("exits[{}].revealed_by", exit.index)),
                    );
                }
            }
            let mut named = Vec::new();
            for (id, _) in &exit.cost.items {
                if id != "target" {
                    named.push(id.clone());
                }
            }
            for cond in &exit.if_ {
                exit_cond_items(cond, &mut named);
            }
            named.sort();
            named.dedup();
            for id in named {
                if !items.contains_key(&id) {
                    diags.push(
                        schema_error(
                            &exit.file,
                            &format!("exit '{}' names unknown item '{id}'", exit.name),
                            &format!("add world/items/{id}.md"),
                        )
                        .with_path(format!("exits[{}]", exit.index)),
                    );
                }
            }
        }
    }
    diags
}

fn exit_cond_items(cond: &Cond, ids: &mut Vec<String>) {
    match cond {
        Cond::HasItem { id, .. } if id != "target" => ids.push(id.clone()),
        Cond::All(inner) | Cond::Any(inner) => {
            for cond in inner {
                exit_cond_items(cond, ids);
            }
        }
        Cond::Not(inner) => exit_cond_items(inner, ids),
        _ => {}
    }
}

pub(super) fn unknown_rule_items(
    rules: &RuleTable,
    items: &BTreeMap<String, ItemCanon>,
) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    for rule in rules.iter() {
        let mut ids = Vec::new();
        for cond in &rule.if_ {
            cond_items(cond, &mut ids);
        }
        if let Some(cost) = &rule.cost {
            for (id, _) in &cost.items {
                if id != "target" {
                    ids.push(id.clone());
                }
            }
        }
        for effect in rule
            .do_
            .iter()
            .chain(rule.success.iter())
            .chain(rule.partial.iter())
            .chain(rule.failure.iter())
        {
            effect_items(effect, &mut ids);
        }
        ids.sort();
        ids.dedup();
        for id in ids {
            if items.contains_key(&id) {
                continue;
            }
            diags.push(unknown_item(
                &rule.file,
                &id,
                &format!("rules[{}]", rule.index),
            ));
        }
    }
    diags
}

fn cond_items(cond: &Cond, ids: &mut Vec<String>) {
    match cond {
        Cond::HasItem { id, .. } | Cond::Wielding { id, .. } => {
            if id != "target" {
                ids.push(id.clone());
            }
        }
        Cond::All(inner) | Cond::Any(inner) => {
            for cond in inner {
                cond_items(cond, ids);
            }
        }
        Cond::Not(inner) => cond_items(inner, ids),
        _ => {}
    }
}

fn effect_items(effect: &EffectTemplate, ids: &mut Vec<String>) {
    match effect {
        EffectTemplate::Item { id, resist, .. } => {
            if id != "target" {
                ids.push(id.clone());
            }
            resist_items(resist.as_ref(), ids);
        }
        EffectTemplate::Wield { id, .. } => {
            if id != "target" {
                ids.push(id.clone());
            }
        }
        EffectTemplate::Meter { resist, .. }
        | EffectTemplate::Flag { resist, .. }
        | EffectTemplate::Narrate { resist, .. } => resist_items(resist.as_ref(), ids),
        _ => {}
    }
}

pub(super) fn find_bare_resist(rules: &RuleTable) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    for rule in rules.iter() {
        let after_do = scan_resist(&rule.do_, false, rule, &mut diags);
        scan_resist(&rule.success, after_do, rule, &mut diags);
        scan_resist(&rule.partial, after_do, rule, &mut diags);
        scan_resist(&rule.failure, after_do, rule, &mut diags);
    }
    diags
}

fn scan_resist(
    effects: &[EffectTemplate],
    mut seen: bool,
    rule: &Rule,
    diags: &mut Vec<Diagnostic>,
) -> bool {
    for effect in effects {
        match effect.resist() {
            Some(ResistSpec::Same) if !seen => {
                diags.push(
                    schema_error(
                        &rule.file,
                        &format!(
                            "rule '{}' uses resist: same before any resist roll",
                            rule.local_id
                        ),
                        "put a resist roll before it, or name the check on this effect",
                    )
                    .with_path(format!("rules[{}]", rule.index)),
                );
            }
            Some(ResistSpec::Roll { resisted, .. }) => {
                seen = true;
                scan_resist(resisted, true, rule, diags);
            }
            _ => {}
        }
    }
    seen
}

fn resist_items(resist: Option<&ResistSpec>, ids: &mut Vec<String>) {
    let Some(ResistSpec::Roll { resisted, .. }) = resist else {
        return;
    };
    for effect in resisted {
        effect_items(effect, ids);
    }
}

pub(super) fn reveal_problems(
    rules: &RuleTable,
    facts: &BTreeMap<String, FactCanon>,
) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    for rule in rules.iter() {
        let mut effects = Vec::new();
        collect_effects(&rule.do_, &mut effects);
        collect_effects(&rule.success, &mut effects);
        collect_effects(&rule.partial, &mut effects);
        collect_effects(&rule.failure, &mut effects);
        for effect in effects {
            let EffectTemplate::Reveal { who, fact } = effect else {
                continue;
            };
            if matches!(who, Who::World) {
                diags.push(
                    schema_error(
                        &rule.file,
                        &format!("rule '{}' reveals to the world", rule.local_id),
                        "name the actor who learns the fact",
                    )
                    .with_path(format!("rules[{}]", rule.index)),
                );
            }
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
                    .with_path(format!("rules[{}]", rule.index)),
                );
            }
        }
    }
    diags
}

fn collect_effects<'a>(effects: &'a [EffectTemplate], out: &mut Vec<&'a EffectTemplate>) {
    for effect in effects {
        out.push(effect);
        if let Some(ResistSpec::Roll { resisted, .. }) = effect.resist() {
            collect_effects(resisted, out);
        }
    }
}

pub(super) fn unknown_checks(
    rules: &RuleTable,
    checks: &BTreeMap<String, CheckDef>,
) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    for rule in rules.iter() {
        let Some(roll) = &rule.roll else { continue };
        if checks.contains_key(&roll.check) {
            continue;
        }
        diags.push(
            schema_error(
                &rule.file,
                &format!(
                    "rule '{}' names check '{}', which is not a check in rules/checks.yaml",
                    rule.local_id, roll.check
                ),
                "add the check or name one that exists",
            )
            .with_path(format!("rules[{}].roll.check", rule.index)),
        );
    }
    diags
}

fn meter_keys(effect: &EffectTemplate) -> Option<&str> {
    match effect {
        EffectTemplate::Meter { key, .. } => Some(key),
        _ => None,
    }
}

pub(super) fn unknown_exprs(
    meters: &BTreeMap<String, MeterDef>,
    cast: &BTreeMap<String, CastCanon>,
    places: &BTreeMap<String, PlaceCanon>,
    rules: &RuleTable,
) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    for rule in rules.iter() {
        let mut exprs = Vec::new();
        push_rule_exprs(rule, &mut exprs);
        for expr in exprs {
            check_expr(
                &rule.file,
                &format!("rules[{}]", rule.index),
                expr,
                meters,
                cast,
                places,
                &mut diags,
            );
        }
    }
    for place in places.values() {
        for exit in &place.exits {
            let mut exprs = Vec::new();
            push_cost(&exit.cost, &mut exprs);
            for cond in &exit.if_ {
                push_cond(cond, &mut exprs);
            }
            for expr in exprs {
                check_expr(
                    &exit.file,
                    &format!("exits[{}]", exit.index),
                    expr,
                    meters,
                    cast,
                    places,
                    &mut diags,
                );
            }
        }
    }
    diags
}

fn check_expr(
    file: &str,
    path: &str,
    expr: &Expr,
    meters: &BTreeMap<String, MeterDef>,
    cast: &BTreeMap<String, CastCanon>,
    places: &BTreeMap<String, PlaceCanon>,
    diags: &mut Vec<Diagnostic>,
) {
    let mut reads = Vec::new();
    expr.meters(&mut reads);
    for (who, key, spot) in reads {
        if matches!(who, Who::World) {
            let mut diag = schema_error(
                file,
                &format!("expression reads 'world.{key}', but the world has no meters"),
                "read the meter from self, target, here, place:<id>, or a cast id",
            )
            .with_path(path);
            if let Some(spot) = spot {
                diag = diag.at_line(spot.line, Some(spot.column));
            }
            diags.push(diag);
            continue;
        }
        if !meters.contains_key(key) {
            let mut diag = schema_error(
                file,
                &format!("expression reads meter '{key}', which is not in rules/meters.yaml"),
                "add the meter or name one that exists",
            )
            .with_path(path);
            if let Some(spot) = spot {
                diag = diag.at_line(spot.line, Some(spot.column));
            }
            diags.push(diag);
        }
        if let Some(name) = unknown_who(who, cast, places) {
            let mut diag = schema_error(
                file,
                &format!("expression names '{name}', which is not a who"),
                "use self, target, here, place:<id>, or a cast id",
            )
            .with_path(path);
            if let Some(spot) = spot {
                diag = diag.at_line(spot.line, Some(spot.column));
            }
            diags.push(diag);
        }
    }
}

fn unknown_who(
    who: &Who,
    cast: &BTreeMap<String, CastCanon>,
    places: &BTreeMap<String, PlaceCanon>,
) -> Option<String> {
    match who {
        Who::Self_ | Who::Target | Who::Here | Who::World => None,
        Who::Place(id) if places.contains_key(id) => None,
        Who::Place(id) => Some(format!("place:{id}")),
        Who::Actor(id) if cast.contains_key(id) => None,
        Who::Actor(id) => Some(id.clone()),
    }
}

fn push_rule_exprs<'a>(rule: &'a Rule, exprs: &mut Vec<&'a Expr>) {
    if let Some(roll) = &rule.roll {
        exprs.push(&roll.target);
        if let Some(band) = &roll.partial_at {
            exprs.push(band);
        }
    }
    if let Some(cost) = &rule.cost {
        push_cost(cost, exprs);
    }
    for cond in &rule.if_ {
        push_cond(cond, exprs);
    }
    for effect in rule
        .do_
        .iter()
        .chain(&rule.success)
        .chain(&rule.partial)
        .chain(&rule.failure)
    {
        push_effect(effect, exprs);
    }
}

fn push_cost<'a>(cost: &'a Cost, exprs: &mut Vec<&'a Expr>) {
    for (_, expr) in cost.meters.iter().chain(cost.items.iter()) {
        exprs.push(expr);
    }
}

fn push_cond<'a>(cond: &'a Cond, exprs: &mut Vec<&'a Expr>) {
    match cond {
        Cond::Meter { value, .. } | Cond::Clock { value, .. } => exprs.push(value),
        Cond::All(inner) | Cond::Any(inner) => {
            for cond in inner {
                push_cond(cond, exprs);
            }
        }
        Cond::Not(inner) => push_cond(inner, exprs),
        _ => {}
    }
}

fn push_effect<'a>(effect: &'a EffectTemplate, exprs: &mut Vec<&'a Expr>) {
    match effect {
        EffectTemplate::Meter {
            add, set, resist, ..
        } => {
            if let Some(expr) = add {
                exprs.push(expr);
            }
            if let Some(expr) = set {
                exprs.push(expr);
            }
            if let Some(spec) = resist {
                push_resist(spec, exprs);
            }
        }
        EffectTemplate::Flag { resist, .. }
        | EffectTemplate::Narrate { resist, .. }
        | EffectTemplate::Item { resist, .. } => {
            if let Some(spec) = resist {
                push_resist(spec, exprs);
            }
        }
        _ => {}
    }
}

fn push_resist<'a>(spec: &'a ResistSpec, exprs: &mut Vec<&'a Expr>) {
    if let ResistSpec::Roll {
        target, resisted, ..
    } = spec
    {
        exprs.push(target);
        for effect in resisted {
            push_effect(effect, exprs);
        }
    }
}

pub(super) fn unknown_meters(
    meters: &BTreeMap<String, MeterDef>,
    checks: &BTreeMap<String, CheckDef>,
    rules: &RuleTable,
) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    for (id, check) in checks {
        if meters.contains_key(&check.stat) {
            continue;
        }
        diags.push(
            schema_error(
                "rules/checks.yaml",
                &format!(
                    "check '{id}' uses stat '{}', which is not in rules/meters.yaml",
                    check.stat
                ),
                "add the meter or name one that exists",
            )
            .with_path(format!("checks.{id}.stat")),
        );
    }
    for rule in rules.iter() {
        for effect in rule
            .do_
            .iter()
            .chain(&rule.success)
            .chain(&rule.partial)
            .chain(&rule.failure)
        {
            let Some(key) = meter_keys(effect) else {
                continue;
            };
            if meters.contains_key(key) {
                continue;
            }
            diags.push(
                schema_error(
                    &rule.file,
                    &format!(
                        "rule '{}' writes meter '{key}', which is not in rules/meters.yaml",
                        rule.local_id
                    ),
                    "add the meter or name one that exists",
                )
                .with_path(format!("rules[{}].do", rule.index)),
            );
        }
    }
    diags
}
