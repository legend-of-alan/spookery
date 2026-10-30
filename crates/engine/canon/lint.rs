//! Cross-file validation of rules, exits, checks, meters, items, facts, and triggers.

use std::collections::{BTreeMap, BTreeSet};

use crate::canon::source::{FileText, Locator, Problem};
use crate::canon::{Canon, CastDef, FactDef, ItemDef, PlaceDef};
use crate::diagnostic::{error, schema_error, warning, Diagnostic};
use crate::error::Reason;
use crate::meter::MeterDef;
use crate::rule::{
    ChangeWatch, Cond, Cost, EffectTemplate, LocationTo, NarrateTo, Owner, ResistSpec, Rule,
    RuleTable, When, Who,
};
use crate::rules::expr::Expr;
use crate::rules::roll::Check;

/// Exits whose `to` is not a place, and facts or items an exit names but the pack does not.
pub(super) fn broken_exits(
    places: &BTreeMap<String, PlaceDef>,
    facts: &BTreeMap<String, FactDef>,
    items: &BTreeMap<String, ItemDef>,
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
                    .at_path(format!("exits[{}].to", exit.index)),
                );
            }
            if exit.hidden && exit.revealed_by.is_none() {
                diags.push(
                    schema_error(
                        &exit.file,
                        &format!("hidden exit '{}' has no revealed_by fact", exit.name),
                        "set revealed_by to a fact id",
                    )
                    .at_path(format!("exits[{}].revealed_by", exit.index)),
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
                        .at_path(format!("exits[{}].revealed_by", exit.index)),
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
                push_exit_cond_items(cond, &mut named);
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
                        .at_path(format!("exits[{}]", exit.index)),
                    );
                }
            }
        }
    }
    diags
}

fn push_exit_cond_items(cond: &Cond, ids: &mut Vec<String>) {
    match cond {
        Cond::HasItem { id, .. } if id != "target" => ids.push(id.clone()),
        Cond::All(inner) | Cond::Any(inner) => {
            for cond in inner {
                push_exit_cond_items(cond, ids);
            }
        }
        Cond::Not(inner) => push_exit_cond_items(inner, ids),
        _ => {}
    }
}

pub(super) fn unknown_rule_items(
    rules: &RuleTable,
    items: &BTreeMap<String, ItemDef>,
) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    for rule in rules.iter() {
        let mut ids = Vec::new();
        for cond in &rule.if_ {
            push_cond_items(cond, &mut ids);
        }
        if let Some(cost) = &rule.cost {
            for (id, _) in &cost.items {
                if id != "target" {
                    ids.push(id.clone());
                }
            }
        }
        rule.visit_templates(|effect| match effect {
            EffectTemplate::Item { id, .. } | EffectTemplate::Wield { id, .. }
                if id != "target" =>
            {
                ids.push(id.clone());
            }
            _ => {}
        });
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

pub(super) fn unknown_item(file: &str, id: &str, path: &str) -> Diagnostic {
    schema_error(
        file,
        &format!("unknown item '{id}'"),
        &format!("add world/items/{id}.md"),
    )
    .at_path(path)
}

fn push_cond_items(cond: &Cond, ids: &mut Vec<String>) {
    match cond {
        Cond::HasItem { id, .. } | Cond::Wielding { id, .. } => {
            if id != "target" {
                ids.push(id.clone());
            }
        }
        Cond::All(inner) | Cond::Any(inner) => {
            for cond in inner {
                push_cond_items(cond, ids);
            }
        }
        Cond::Not(inner) => push_cond_items(inner, ids),
        _ => {}
    }
}

pub(super) fn resist_same_without_roll(rules: &RuleTable) -> Vec<Diagnostic> {
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
                    .at_path(format!("rules[{}]", rule.index)),
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

pub(super) fn reveal_problems(
    rules: &RuleTable,
    facts: &BTreeMap<String, FactDef>,
) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    for rule in rules.iter() {
        rule.visit_templates(|effect| {
            let EffectTemplate::Reveal { who, fact } = effect else {
                return;
            };
            if matches!(who, Who::World) {
                diags.push(
                    schema_error(
                        &rule.file,
                        &format!("rule '{}' reveals to the world", rule.local_id),
                        "name the actor who learns the fact",
                    )
                    .at_path(format!("rules[{}]", rule.index)),
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
                    .at_path(format!("rules[{}]", rule.index)),
                );
            }
        });
    }
    diags
}

pub(super) fn unknown_checks(
    rules: &RuleTable,
    checks: &BTreeMap<String, Check>,
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
            .at_path(format!("rules[{}].roll.check", rule.index)),
        );
    }
    diags
}

pub(super) fn unknown_reads(
    meters: &BTreeMap<String, MeterDef>,
    cast: &BTreeMap<String, CastDef>,
    places: &BTreeMap<String, PlaceDef>,
    rules: &RuleTable,
) -> Vec<Diagnostic> {
    let known = ExprScope {
        meters,
        cast,
        places,
    };
    let mut diags = Vec::new();
    for rule in rules.iter() {
        let mut exprs = Vec::new();
        push_rule_exprs(rule, &mut exprs);
        let path = format!("rules[{}]", rule.index);
        known.push_unknown_reads(&FileText::named(&rule.file), &path, &exprs, &mut diags);
    }
    for place in places.values() {
        for exit in &place.exits {
            let mut exprs = Vec::new();
            push_cost(&exit.cost, &mut exprs);
            for cond in &exit.if_ {
                push_cond(cond, &mut exprs);
            }
            let path = format!("exits[{}]", exit.index);
            known.push_unknown_reads(&FileText::named(&exit.file), &path, &exprs, &mut diags);
        }
    }
    diags
}

/// What an expression may read: the pack's meters, and the cast and places it may name.
struct ExprScope<'a> {
    meters: &'a BTreeMap<String, MeterDef>,
    cast: &'a BTreeMap<String, CastDef>,
    places: &'a BTreeMap<String, PlaceDef>,
}

impl ExprScope<'_> {
    /// The unknown reads in `exprs`, at `path` in `source` and at each read's spot.
    fn push_unknown_reads(
        &self,
        source: &FileText,
        path: &str,
        exprs: &[&Expr],
        diags: &mut Vec<Diagnostic>,
    ) {
        for expr in exprs {
            for (who, key, spot) in expr.meter_reads() {
                let at = Locator::Spot(spot);
                let refuse = |problem, hint| {
                    Problem {
                        source,
                        path,
                        at,
                        message: problem,
                        hint,
                    }
                    .into_diagnostic()
                };
                if matches!(who, Who::World) {
                    diags.push(refuse(
                        format!("expression reads 'world.{key}', but the world has no meters"),
                        "read the meter from self, target, here, place:<id>, or a cast id",
                    ));
                    continue;
                }
                if !self.meters.contains_key(key) {
                    diags.push(refuse(
                        format!(
                            "expression reads meter '{key}', which is not in rules/meters.yaml"
                        ),
                        "add the meter or name one that exists",
                    ));
                }
                if let Some(name) = unknown_who(who, self.cast, self.places) {
                    diags.push(refuse(
                        format!("expression names '{name}', which is not a who"),
                        "use self, target, here, place:<id>, or a cast id",
                    ));
                }
            }
        }
    }
}

fn unknown_who(
    who: &Who,
    cast: &BTreeMap<String, CastDef>,
    places: &BTreeMap<String, PlaceDef>,
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
    rule.visit_templates(|effect| {
        if let EffectTemplate::Meter { add, set, .. } = effect {
            if let Some(expr) = add {
                exprs.push(expr);
            }
            if let Some(expr) = set {
                exprs.push(expr);
            }
        }
        if let Some(ResistSpec::Roll { target, .. }) = effect.resist() {
            exprs.push(target);
        }
    });
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

pub(super) fn unknown_meters(
    meters: &BTreeMap<String, MeterDef>,
    checks: &BTreeMap<String, Check>,
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
                    "check '{id}' uses meter '{}', which is not in rules/meters.yaml",
                    check.stat
                ),
                "add the meter or name one that exists",
            )
            .at_path(format!("checks.{id}.meter")),
        );
    }
    for rule in rules.iter() {
        for effect in rule.top_templates() {
            let EffectTemplate::Meter { key, .. } = effect else {
                continue;
            };
            if meters.contains_key(key.as_str()) {
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
                .at_path(format!("rules[{}].do", rule.index)),
            );
        }
    }
    diags
}

/// Static warnings and trigger-reach errors on a loaded `Canon`.
pub(crate) fn check_canon(canon: &Canon, now: Option<&crate::model::State>) -> Vec<Diagnostic> {
    let mut diags = unreachable_places(canon);
    diags.extend(teleport_in_verbs(canon));
    diags.extend(trigger_cycles(canon));
    diags.extend(trigger_leaks(canon, now));
    diags
}

/// Places no exit path reaches from the start, including hidden and locked exits.
fn unreachable_places(canon: &Canon) -> Vec<Diagnostic> {
    let mut seen = BTreeSet::new();
    let mut queue = vec![canon.start_place.clone()];
    while let Some(place) = queue.pop() {
        if !seen.insert(place.clone()) {
            continue;
        }
        if let Some(node) = canon.places.get(&place) {
            for exit in &node.exits {
                queue.push(exit.to.clone());
            }
        }
    }
    let mut diags = Vec::new();
    for id in canon.places.keys() {
        if seen.contains(id) {
            continue;
        }
        diags.push(
            warning(
                "world/places",
                &format!("place '{id}' has no path from {}", canon.start_place),
                "add an exit that reaches it",
            )
            .at_path(id.clone()),
        );
    }
    diags
}

/// `teleport: true` in the shared verbs file is a warning. Author rules may teleport.
fn teleport_in_verbs(canon: &Canon) -> Vec<Diagnostic> {
    canon
        .rules
        .iter()
        .filter(|rule| {
            rule.owner == Owner::Verb
                && rule
                    .top_templates()
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

/// Change rules that set each other's watched values.
fn trigger_cycles(canon: &Canon) -> Vec<Diagnostic> {
    let rules: Vec<&Rule> = canon
        .rules
        .iter()
        .filter(|rule| matches!(rule.when, When::Change(_)))
        .collect();
    let written: Vec<Vec<Write>> = rules
        .iter()
        .map(|rule| {
            let mut marks = Vec::new();
            rule.visit_templates(|effect| push_writes(effect, &mut marks));
            marks
        })
        .collect();
    let mut next = vec![Vec::new(); rules.len()];
    for (from, marks) in written.iter().enumerate() {
        for (to, rule) in rules.iter().enumerate() {
            if from == to {
                continue;
            }
            let When::Change(watch) = &rule.when else {
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
fn trigger_leaks(canon: &Canon, now: Option<&crate::model::State>) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    for rule in canon.rules.iter() {
        let When::Change(watch) = &rule.when else {
            continue;
        };
        let sight = sight_of(canon, now, watch);
        let mut leak = false;
        rule.visit_templates(|effect| {
            if reach_of(effect).is_some_and(|reach| is_too_wide(&sight, &reach)) {
                leak = true;
            }
        });
        if !leak {
            continue;
        }
        diags.push(
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
    diags
}

#[derive(Clone)]
enum Write {
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
    Place,
}

enum Sight {
    Subject,
    Room,
    Owners(Vec<String>),
    Everyone,
}

fn push_writes(effect: &EffectTemplate, out: &mut Vec<Write>) {
    match effect {
        EffectTemplate::Flag { key, value, .. } => {
            out.push(Write::Flag {
                key: key.clone(),
                value: *value,
            });
        }
        EffectTemplate::Meter { key, .. } => out.push(Write::Meter(key.clone())),
        EffectTemplate::Item { id, add, .. } => {
            if *add > 0 {
                out.push(Write::Item {
                    id: id.clone(),
                    gained: true,
                });
            } else if *add < 0 {
                out.push(Write::Item {
                    id: id.clone(),
                    gained: false,
                });
            }
        }
        EffectTemplate::Clock { id, add, .. } if add.is_some() => {
            out.push(Write::Clock(id.clone()));
        }
        EffectTemplate::Location {
            to: LocationTo::Place(id),
            ..
        } => out.push(Write::Arrive(id.clone())),
        EffectTemplate::Reveal { fact, .. } => out.push(Write::Knows(fact.clone())),
        EffectTemplate::Memory { fact_ids, .. } => {
            for fact in fact_ids {
                out.push(Write::Knows(fact.clone()));
            }
        }
        _ => {}
    }
}

fn sets_watch(watch: &ChangeWatch, rule: &Rule, mark: &Write) -> bool {
    match (watch, mark) {
        (
            ChangeWatch::Flag { key, becomes },
            Write::Flag {
                key: written,
                value,
            },
        ) => key == written && becomes == value,
        (ChangeWatch::Meter { key, .. }, Write::Meter(written)) => key == written,
        (
            ChangeWatch::Item { id, gained },
            Write::Item {
                id: written,
                gained: up,
            },
        ) => id == written && gained == up,
        (ChangeWatch::Clock { id }, Write::Clock(written)) => id == written,
        (ChangeWatch::Arrive { place, .. }, Write::Arrive(dest)) => {
            rule.watched_place(place) == dest
        }
        (ChangeWatch::Knows { fact }, Write::Knows(written)) => fact == written,
        _ => false,
    }
}

fn sight_of(canon: &Canon, now: Option<&crate::model::State>, watch: &ChangeWatch) -> Sight {
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
    let mut world = false;
    let mut actor = false;
    for rule in canon.rules.iter() {
        rule.visit_templates(|effect| {
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

fn reach_of(effect: &EffectTemplate) -> Option<Reach> {
    match effect {
        EffectTemplate::Narrate { to, .. } => Some(match to {
            NarrateTo::Self_ => Reach::Subject,
            NarrateTo::Here => Reach::Room,
            NarrateTo::Actors(ids) => Reach::Named(ids.clone()),
        }),
        EffectTemplate::Note { to, place, .. } => {
            if place.is_some() {
                Some(Reach::Place)
            } else {
                Some(Reach::Named(to.clone()))
            }
        }
        _ => None,
    }
}

fn is_too_wide(sight: &Sight, reach: &Reach) -> bool {
    match sight {
        Sight::Everyone => false,
        Sight::Subject => !matches!(reach, Reach::Subject),
        Sight::Room => !matches!(reach, Reach::Subject | Reach::Room),
        Sight::Owners(owners) => match reach {
            Reach::Subject => false,
            Reach::Named(ids) => ids.iter().any(|id| !owners.iter().any(|owner| owner == id)),
            Reach::Room | Reach::Place => true,
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
