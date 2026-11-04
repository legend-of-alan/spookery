//! Cross-file validation of rules, exits, checks, meters, items, facts, and triggers.

use std::collections::{BTreeMap, BTreeSet};

use crate::canon::source::{FileText, Locator, Problem};
use crate::canon::{Canon, CastDef, FactDef, ItemDef, PlaceDef};
use crate::diagnostic::{error, schema_error, warning, Diagnostic};
use crate::model::meter::{self, MeterDef};
use crate::rules::effect::Holder;
use crate::rules::expr::Expr;
use crate::rules::roll::Check;
use crate::rules::rule::{
    ChangeWatch, Cond, Cost, EffectTemplate, LocationTo, NarrateTo, Owner, ResistSpec, Rule,
    RuleTable, When, Who,
};
use crate::Reason;

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

pub(super) fn unknown_reads(scope: &MeterScope, rules: &RuleTable) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    for rule in rules.iter() {
        let mut exprs = Vec::new();
        push_rule_exprs(rule, &mut exprs);
        let path = format!("rules[{}]", rule.index);
        let source = FileText::named(&rule.file);
        scope.push_unknown_reads(&source, &path, fixed_self(rule), &exprs, &mut diags);
    }
    for place in scope.places.values() {
        for exit in &place.exits {
            let mut exprs = Vec::new();
            push_cost(&exit.cost, &mut exprs);
            for cond in &exit.if_ {
                push_cond(cond, &mut exprs);
            }
            let path = format!("exits[{}]", exit.index);
            let source = FileText::named(&exit.file);
            scope.push_unknown_reads(&source, &path, None, &exprs, &mut diags);
        }
    }
    diags
}

/// Meter effects and conditions whose holder is fixed by the pack and does not carry the meter.
pub(super) fn unknown_carriers(scope: &MeterScope, rules: &RuleTable) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    for rule in rules.iter() {
        let mut named = Vec::new();
        for cond in &rule.if_ {
            push_cond_meters(cond, &mut named);
        }
        rule.visit_templates(|effect| {
            if let EffectTemplate::Meter { who, key, .. } = effect {
                named.push((who, key.as_str()));
            }
        });
        for (who, key) in named {
            let Some(why) = scope.why_not_carried(fixed_self(rule), who, key) else {
                continue;
            };
            diags.push(
                schema_error(
                    &rule.file,
                    &format!("rule '{}' names meter '{key}', but {why}", rule.local_id),
                    "declare the meter on that holder, or name a holder that carries it",
                )
                .at_path(format!("rules[{}]", rule.index)),
            );
        }
    }
    diags
}

fn push_cond_meters<'a>(cond: &'a Cond, out: &mut Vec<(&'a Who, &'a str)>) {
    match cond {
        Cond::Meter { who, key, .. } => out.push((who, key)),
        Cond::All(inner) | Cond::Any(inner) => {
            for cond in inner {
                push_cond_meters(cond, out);
            }
        }
        Cond::Not(inner) => push_cond_meters(inner, out),
        _ => {}
    }
}

/// The actor `self` always is on `rule`: the cast member, on its own habit.
fn fixed_self(rule: &Rule) -> Option<&str> {
    match (&rule.owner, &rule.when) {
        (Owner::Cast(id), When::Habit) => Some(id),
        _ => None,
    }
}

/// The meters each holder carries, and the cast and places a `who` may name.
pub(super) struct MeterScope<'a> {
    pub meters: &'a BTreeMap<String, MeterDef>,
    pub world_meters: &'a BTreeMap<String, MeterDef>,
    pub cast: &'a BTreeMap<String, CastDef>,
    pub places: &'a BTreeMap<String, PlaceDef>,
}

impl MeterScope<'_> {
    /// Why meter `key` on `who` can never be read or written, or `None` when it may be.
    /// A holder known only at play time is not guessed: only its key is checked.
    fn why_not_carried(&self, fixed_self: Option<&str>, who: &Who, key: &str) -> Option<String> {
        let actor = match who {
            Who::World if self.world_meters.contains_key(key) => return None,
            Who::World => return Some(format!("the world carries no meter '{key}'")),
            Who::Actor(id) if self.cast.contains_key(id) => Some(id.as_str()),
            Who::Self_ => fixed_self,
            _ => None,
        };
        let def = self.meters.get(key);
        match (actor, def) {
            (Some(id), def) if !def.is_some_and(|def| def.is_on_actor(id)) => {
                Some(match def.and_then(|def| def.carrier.as_ref()) {
                    Some(Holder::Actor(carrier)) => format!("only {carrier} carries meter '{key}'"),
                    _ => format!("{id} carries no meter '{key}'"),
                })
            }
            (None, None) => Some(format!(
                "meter '{key}' is not in rules/meters.yaml or a cast file"
            )),
            _ => None,
        }
    }

    /// The unknown reads in `exprs`, at `path` in `source` and at each read's spot.
    fn push_unknown_reads(
        &self,
        source: &FileText,
        path: &str,
        fixed_self: Option<&str>,
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
                if let Some(why) = self.why_not_carried(fixed_self, who, key) {
                    diags.push(refuse(
                        format!("expression reads meter '{key}', but {why}"),
                        "declare the meter on that holder, or read one that exists",
                    ));
                }
                if let Some(name) = unknown_who(who, self.cast, self.places) {
                    diags.push(refuse(
                        format!("expression names '{name}', which is not a who"),
                        "use self, target, here, world, place:<id>, or a cast id",
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
        Cond::Meter { value, .. } => exprs.push(value),
        Cond::All(inner) | Cond::Any(inner) => {
            for cond in inner {
                push_cond(cond, exprs);
            }
        }
        Cond::Not(inner) => push_cond(inner, exprs),
        _ => {}
    }
}

/// Check stats that are not meters every actor carries.
pub(super) fn unknown_meters(
    meters: &BTreeMap<String, MeterDef>,
    checks: &BTreeMap<String, Check>,
) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    for (id, check) in checks {
        if meters
            .get(&check.stat)
            .is_some_and(|def| def.carrier.is_none())
        {
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
    diags
}

/// Check the save against the pack without changing it. Refuse every meter,
/// item and wielded slot the pack does not define, every meter the pack gives a holder
/// that the holder lacks, every meter outside its rules and every empty id or zero count.
pub fn check_save(canon: &Canon, state: &crate::model::State) -> Result<(), Vec<Diagnostic>> {
    let mut diags = Vec::new();
    for (id, actor) in &state.actors {
        let holder = Holder::Actor(id.clone());
        let path = format!("actors.{id}.meters");
        check_meters(canon, &holder, &actor.meters, &path, &mut diags);
    }
    check_meters(
        canon,
        &Holder::World,
        &state.world_meters,
        "world_meters",
        &mut diags,
    );
    for (id, actor) in &state.actors {
        for (item, count) in &actor.inventory {
            if item.is_empty() {
                diags.push(
                    schema_error(
                        "state/save.yaml",
                        &format!("actor '{id}' lists an empty item id"),
                        "give the item a non-empty id",
                    )
                    .at_path(format!("actors.{id}.inventory")),
                );
            }
            if *count == 0 {
                diags.push(
                    schema_error(
                        "state/save.yaml",
                        &format!("actor '{id}' lists item '{item}' with a count of 0"),
                        "omit an item the actor does not hold",
                    )
                    .at_path(format!("actors.{id}.inventory.{item}")),
                );
            }
        }
    }
    for (place, live) in &state.places {
        for (item, count) in &live.items {
            if *count == 0 {
                diags.push(
                    schema_error(
                        "state/save.yaml",
                        &format!("place '{place}' lists item '{item}' with a count of 0"),
                        "omit an item that is not there",
                    )
                    .at_path(format!("places.{place}.items.{item}")),
                );
            }
        }
    }
    for (actor_id, actor) in &state.actors {
        for item in actor.inventory.keys() {
            if !canon.items.contains_key(item) {
                diags.push(unknown_item(
                    "state/save.yaml",
                    item,
                    &format!("actors.{actor_id}.inventory.{item}"),
                ));
            }
        }
        if let Some(wielded) = actor.wielded.as_deref() {
            if !canon.items.contains_key(wielded) {
                diags.push(unknown_item(
                    "state/save.yaml",
                    wielded,
                    &format!("actors.{actor_id}.wielded"),
                ));
            } else if !canon
                .items
                .get(wielded)
                .is_some_and(|item: &ItemDef| item.tags.iter().any(|tag| tag == "weapon"))
            {
                diags.push(
                    schema_error(
                        "state/save.yaml",
                        &format!("actor '{actor_id}' wields '{wielded}', which is not a weapon"),
                        "wield an item tagged weapon, or leave the slot empty",
                    )
                    .at_path(format!("actors.{actor_id}.wielded")),
                );
            } else if actor.item_count(wielded) == 0 {
                diags.push(
                    schema_error(
                        "state/save.yaml",
                        &format!("actor '{actor_id}' wields '{wielded}' without holding it"),
                        "put the item in inventory or clear wielded",
                    )
                    .at_path(format!("actors.{actor_id}.wielded")),
                );
            }
        }
    }
    for (place, live) in &state.places {
        for item in live.items.keys() {
            if !canon.items.contains_key(item) {
                diags.push(unknown_item(
                    "state/save.yaml",
                    item,
                    &format!("places.{place}.items.{item}"),
                ));
            }
        }
    }
    if diags.is_empty() {
        Ok(())
    } else {
        Err(diags)
    }
}

/// The live meters of `holder`, at `path` in the save, against the meters the pack gives it.
fn check_meters(
    canon: &Canon,
    holder: &Holder,
    live: &BTreeMap<String, meter::MeterLive>,
    path: &str,
    diags: &mut Vec<Diagnostic>,
) {
    let (name, defs) = match holder {
        Holder::Actor(id) => (format!("actor '{id}'"), &canon.meters),
        _ => ("the world".to_string(), &canon.world_meters),
    };
    for (key, def) in defs {
        let carried = match holder {
            Holder::Actor(id) => def.is_on_actor(id),
            _ => true,
        };
        if carried && !live.contains_key(key) {
            diags.push(
                schema_error(
                    "state/save.yaml",
                    &format!("{name} lacks meter '{key}', which the pack gives it"),
                    "run spook migrate to add the meter",
                )
                .at_path(format!("{path}.{key}")),
            );
        }
    }
    for (key, meter) in live {
        let Some(def) = canon.meter_def(holder, key) else {
            diags.push(
                schema_error(
                    "state/save.yaml",
                    &format!("{name} lists meter '{key}', which the pack does not give it"),
                    "remove the meter or declare it for this holder",
                )
                .at_path(format!("{path}.{key}")),
            );
            continue;
        };
        if !meter::is_storable(def, meter.base, meter.offset) {
            diags.push(
                schema_error(
                    "state/save.yaml",
                    &format!("{name} meter '{key}' is outside the rules"),
                    "set base and offset so the stored pair fits the meter",
                )
                .at_path(format!("{path}.{key}")),
            );
        }
    }
}

/// Static warnings and trigger-reach errors on a loaded `Canon`.
pub(crate) fn check_canon(canon: &Canon) -> Vec<Diagnostic> {
    let mut diags = check_reachable_places(canon);
    diags.extend(check_teleport_verbs(canon));
    diags.extend(check_trigger_cycles(canon));
    diags.extend(check_trigger_leaks(canon));
    diags.extend(check_world_change_subjects(canon));
    diags
}

/// Places no exit path reaches from the start, including hidden and locked exits.
fn check_reachable_places(canon: &Canon) -> Vec<Diagnostic> {
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
fn check_teleport_verbs(canon: &Canon) -> Vec<Diagnostic> {
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
fn check_trigger_cycles(canon: &Canon) -> Vec<Diagnostic> {
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
fn check_trigger_leaks(canon: &Canon) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
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
    Arrive(String),
    Knows(String),
}

/// Who a narrate or note reaches: the subject, the room, or anyone else it names.
enum Reach {
    Subject,
    Room,
    Others,
}

/// Who may see a watched change. A meter or flag change on a watch counts as the room's,
/// because a rule that narrates it is the pack choosing to tell the room. A world change has
/// no room, so the pack tells whoever it names.
enum Sight {
    Subject,
    Room,
    World,
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
        (ChangeWatch::Arrive { place, .. }, Write::Arrive(dest)) => {
            rule.watched_place(place) == dest
        }
        (ChangeWatch::Knows { fact }, Write::Knows(written)) => fact == written,
        _ => false,
    }
}

fn sight_of(canon: &Canon, watch: &ChangeWatch) -> Sight {
    if watches_world(canon, watch) {
        return Sight::World;
    }
    match watch {
        ChangeWatch::Knows { .. } => Sight::Subject,
        ChangeWatch::Arrive { .. }
        | ChangeWatch::Leave { .. }
        | ChangeWatch::Meter { .. }
        | ChangeWatch::Flag { .. }
        | ChangeWatch::Item { .. } => Sight::Room,
    }
}

fn reach_of(effect: &EffectTemplate) -> Option<Reach> {
    match effect {
        EffectTemplate::Narrate { to, .. } => Some(match to {
            NarrateTo::Self_ => Reach::Subject,
            NarrateTo::Here => Reach::Room,
            NarrateTo::Actors(_) | NarrateTo::Present => Reach::Others,
        }),
        EffectTemplate::Note { .. } => Some(Reach::Others),
        _ => None,
    }
}

fn is_too_wide(sight: &Sight, reach: &Reach) -> bool {
    match sight {
        Sight::Subject => !matches!(reach, Reach::Subject),
        Sight::Room => !matches!(reach, Reach::Subject | Reach::Room),
        Sight::World => false,
    }
}

/// Whether `watch` can match a change the world holds: a meter in `world/meters.yaml`, or a
/// flag some rule sets with `who: world`.
fn watches_world(canon: &Canon, watch: &ChangeWatch) -> bool {
    match watch {
        ChangeWatch::Meter { key, .. } => canon.world_meters.contains_key(key),
        ChangeWatch::Flag { key, .. } => canon.rules.iter().any(|rule| {
            let mut sets = false;
            rule.visit_templates(|effect| {
                if let EffectTemplate::Flag {
                    who: Who::World,
                    key: written,
                    ..
                } = effect
                {
                    sets |= written == key;
                }
            });
            sets
        }),
        _ => false,
    }
}

/// A change rule on a world change runs once, for no one, so it has no `self` and no `here`.
fn check_world_change_subjects(canon: &Canon) -> Vec<Diagnostic> {
    canon
        .rules
        .iter()
        .filter(|rule| matches!(&rule.when, When::Change(watch) if watches_world(canon, watch)))
        .filter(|rule| names_subject(rule))
        .map(|rule| {
            schema_error(
                &rule.file,
                &format!(
                    "rule '{}' watches a world change, which has no self and no here",
                    rule.local_id
                ),
                "name the actor, or world",
            )
            .at_path(format!("rules[{}]", rule.index))
        })
        .collect()
}

/// Whether `rule` needs a subject: `self` or `here` as a who or a narrate's `to`, a location
/// effect, which moves the subject, or a roll, which is the subject's.
fn names_subject(rule: &Rule) -> bool {
    let mut whos = Vec::new();
    for cond in &rule.if_ {
        push_cond_whos(cond, &mut whos);
    }
    let mut exprs = Vec::new();
    push_rule_exprs(rule, &mut exprs);
    for expr in exprs {
        whos.extend(expr.meter_reads().into_iter().map(|(who, _, _)| who));
    }
    let mut needs = rule.roll.is_some();
    rule.visit_templates(|effect| {
        match effect {
            EffectTemplate::Meter { who, .. }
            | EffectTemplate::Flag { who, .. }
            | EffectTemplate::Memory { who, .. }
            | EffectTemplate::Item { who, .. }
            | EffectTemplate::Reveal { who, .. }
            | EffectTemplate::Wield { who, .. }
            | EffectTemplate::Unwield { who } => whos.push(who),
            EffectTemplate::Narrate { to, .. } => {
                needs |= matches!(to, NarrateTo::Self_ | NarrateTo::Here);
            }
            EffectTemplate::Location { .. } => needs = true,
            EffectTemplate::Note { .. } => {}
        }
        if let Some(ResistSpec::Roll { who, .. }) = effect.resist() {
            whos.push(who);
        }
    });
    needs || whos.iter().any(|who| matches!(who, Who::Self_ | Who::Here))
}

fn push_cond_whos<'a>(cond: &'a Cond, out: &mut Vec<&'a Who>) {
    match cond {
        Cond::HasItem { who, .. }
        | Cond::Meter { who, .. }
        | Cond::Flag { who, .. }
        | Cond::At { who, .. }
        | Cond::Knows { who, .. }
        | Cond::Wielding { who, .. } => out.push(who),
        Cond::All(inner) | Cond::Any(inner) => {
            for cond in inner {
                push_cond_whos(cond, out);
            }
        }
        Cond::Not(inner) => push_cond_whos(inner, out),
        Cond::Hour { .. } | Cond::Players { .. } | Cond::NoteExists(_) => {}
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
