//! Cross-file validation of rules, exits, checks, meters, items, facts, and triggers.

use std::collections::{BTreeMap, BTreeSet};

use crate::canon::source::{FileText, Locator, Problem};
use crate::canon::{declared_flags, Canon, CastDef, FactDef, ItemDef, PlaceDef, StoryDef, OPEN};
use crate::diagnostic::{error, schema_error, warning, Diagnostic};
use crate::entity;
use crate::rules::expr::Expr;
use crate::rules::grammar;
use crate::rules::roll::Check;
use crate::rules::rule::{
    ChangeWatch, Cond, Cost, EffectTemplate, LocationTo, NarrateTo, Owner, ResistSpec, Rule,
    RuleTable, When,
};
use crate::state::component::meter::{self, MeterDef};
use crate::state::component::modifier::{self, ModifierDef};
use crate::state::id::{ActorId, PlaceId, StoryId};
use crate::state::{self, nested_map};
use crate::Reason;

/// Exits whose `to` is not a place, and facts or items an exit names but the pack does not.
pub(super) fn broken_exits(
    places: &BTreeMap<PlaceId, PlaceDef>,
    facts: &BTreeMap<String, FactDef>,
    items: &BTreeMap<String, ItemDef>,
) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    for place in places.values() {
        for exit in &place.exits {
            if !places.contains_key(exit.to.as_str()) {
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
                cond.visit(|cond| match cond {
                    Cond::HasItem { id, .. } if id != "target" => named.push(id.clone()),
                    _ => {}
                });
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

pub(super) fn unknown_rule_items(
    rules: &RuleTable,
    items: &BTreeMap<String, ItemDef>,
) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    for rule in rules.iter() {
        let mut ids = Vec::new();
        for cond in &rule.if_ {
            cond.visit(|cond| match cond {
                Cond::HasItem { id, .. } | Cond::Wielding { id, .. } if id != "target" => {
                    ids.push(id.clone());
                }
                _ => {}
            });
        }
        for cost in rule.costs() {
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

/// Modifier effects naming a modifier `rules/modifiers.yaml` does not define.
pub(super) fn unknown_modifiers(
    rules: &RuleTable,
    modifiers: &BTreeMap<String, ModifierDef>,
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

pub(super) fn unknown_item(file: &str, id: &str, path: &str) -> Diagnostic {
    schema_error(
        file,
        &format!("unknown item '{id}'"),
        &format!("add world/items/{id}.md"),
    )
    .at_path(path)
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
            let EffectTemplate::Reveal { of, fact } = effect else {
                return;
            };
            if matches!(of, grammar::Object::Global) {
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

pub(super) fn unknown_reads(scope: &EntityScope, rules: &RuleTable) -> Vec<Diagnostic> {
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
                cond.visit(|cond| {
                    if let Cond::Meter { value, .. } = cond {
                        exprs.push(value);
                    }
                });
            }
            let path = format!("exits[{}]", exit.index);
            let source = FileText::named(&exit.file);
            scope.push_unknown_reads(&source, &path, None, &exprs, &mut diags);
        }
    }
    diags
}

/// Meter and flag effects and conditions whose entity is fixed by the pack and does not carry
/// the meter or declare the flag. The world's flags are free names, so they are not checked.
pub(super) fn unknown_carriers(scope: &EntityScope, rules: &RuleTable) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    for rule in rules.iter() {
        let mut named = Vec::new();
        for cond in &rule.if_ {
            cond.visit(|cond| match cond {
                Cond::Meter { of, key, .. } => named.push(("meter", of, key.as_str())),
                Cond::Flag { of, key, .. } => named.push(("flag", of, key.as_str())),
                _ => {}
            });
        }
        rule.visit_templates(|effect| match effect {
            EffectTemplate::Meter { of, key, .. } => named.push(("meter", of, key.as_str())),
            EffectTemplate::Flag { of, key, .. } => named.push(("flag", of, key.as_str())),
            _ => {}
        });
        let fixed = fixed_self(rule);
        for (kind, of, key) in named {
            let why = match kind {
                "meter" => scope.why_not_carried(fixed, of, key),
                _ => scope.why_undeclared(fixed, of, key),
            };
            let Some(why) = why else {
                continue;
            };
            diags.push(
                schema_error(
                    &rule.file,
                    &format!("rule '{}' names {kind} '{key}', but {why}", rule.local_id),
                    "declare it on that entity, or name an entity that has it",
                )
                .at_path(format!("rules[{}]", rule.index)),
            );
        }
    }
    diags
}

/// The actor `self` always is on `rule`: the cast member, on its own habit.
fn fixed_self(rule: &Rule) -> Option<&str> {
    match (&rule.owner, &rule.when) {
        (Owner::Cast(id), When::Habit) => Some(id),
        _ => None,
    }
}

/// The meters each entity carries, and the cast, places and stories an `of` may name.
pub(super) struct EntityScope<'a> {
    pub meters: &'a BTreeMap<String, MeterDef>,
    pub world_meters: &'a BTreeMap<String, MeterDef>,
    pub cast: &'a BTreeMap<ActorId, CastDef>,
    pub places: &'a BTreeMap<PlaceId, PlaceDef>,
    pub stories: &'a BTreeMap<StoryId, StoryDef>,
}

impl EntityScope<'_> {
    /// The one entity `of` names whatever the round: `world`, a place, a story, a cast id, or
    /// `self` on a cast member's own habit. `None` when only play time knows it.
    fn fixed_entity(&self, fixed_self: Option<&str>, of: &grammar::Object) -> Option<entity::Id> {
        match of {
            grammar::Object::Global => Some(entity::Id::Global),
            grammar::Object::Place(id) => Some(entity::Id::Place(id.clone())),
            grammar::Object::Story(id) => Some(entity::Id::Story(id.clone())),
            grammar::Object::Actor(id) if self.cast.contains_key(id) => {
                Some(entity::Id::Actor(id.clone()))
            }
            grammar::Object::Self_ => fixed_self.map(|id| entity::Id::Actor(id.into())),
            _ => None,
        }
    }

    /// Why flag `key` of the owner `of` names can never be set or tested, or `None` when it may be.
    fn why_undeclared(
        &self,
        fixed_self: Option<&str>,
        of: &grammar::Object,
        key: &str,
    ) -> Option<String> {
        if let grammar::Object::Exit(exit) = of {
            return if self.unknown_object(of).is_some() {
                Some(format!("{exit} is not an exit"))
            } else if key != OPEN {
                Some(format!("an exit carries only the flag '{OPEN}'"))
            } else {
                let door = (self.places.get(&exit.place)).is_some_and(|at| at.has_door(&exit.exit));
                (!door).then(|| format!("{exit} has no if, so it is always open"))
            };
        }
        let id = self.fixed_entity(fixed_self, of)?;
        let flags = declared_flags(self.cast, self.places, self.stories, id.as_ref())?;
        let declared = flags.iter().any(|flag| flag == key);
        (!declared).then(|| format!("{id} does not declare it"))
    }

    /// Why meter `key` of the owner `of` names can never be read or written, or `None` when it
    /// may be.
    /// An entity known only at play time is not guessed: only its key is checked.
    fn why_not_carried(
        &self,
        fixed_self: Option<&str>,
        of: &grammar::Object,
        key: &str,
    ) -> Option<String> {
        let def = self.meters.get(key);
        if let grammar::Object::Exit(exit) = of {
            return Some(format!("{exit} carries no meter '{key}'"));
        }
        match self.fixed_entity(fixed_self, of) {
            Some(entity::Id::Global) if self.world_meters.contains_key(key) => None,
            Some(entity::Id::Global) => Some(format!("the world carries no meter '{key}'")),
            Some(id) if !def.is_some_and(|def| def.is_on(id.as_ref())) => {
                Some(match def.and_then(|def| def.carrier.as_ref()) {
                    Some(carrier) => format!("only {carrier} carries meter '{key}'"),
                    None => format!("{id} carries no meter '{key}'"),
                })
            }
            None if def.is_none() => Some(format!("no file declares meter '{key}'")),
            _ => None,
        }
    }

    /// The name `of` gives when no place, story or cast member has it.
    fn unknown_object(&self, of: &grammar::Object) -> Option<String> {
        match of {
            grammar::Object::Self_
            | grammar::Object::Target
            | grammar::Object::Here
            | grammar::Object::Global => None,
            grammar::Object::Place(id) if self.places.contains_key(id) => None,
            grammar::Object::Place(id) => Some(format!("place:{id}")),
            grammar::Object::Story(id) if self.stories.contains_key(id) => None,
            grammar::Object::Story(id) => Some(format!("story:{id}")),
            grammar::Object::Exit(exit) => {
                let exits = self.places.get(&exit.place).map(|at| at.exits.as_slice());
                let known = exits.is_some_and(|exits| exits.iter().any(|at| at.name == exit.exit));
                (!known).then(|| exit.to_string())
            }
            grammar::Object::Actor(id) if self.cast.contains_key(id) => None,
            grammar::Object::Actor(id) => Some(id.to_string()),
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
            for (of, key, spot) in expr.meter_reads() {
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
                if let Some(why) = self.why_not_carried(fixed_self, of, key) {
                    diags.push(refuse(
                        format!("expression reads meter '{key}', but {why}"),
                        "declare the meter on that entity, or read one that exists",
                    ));
                }
                if let Some(name) = self.unknown_object(of) {
                    diags.push(refuse(
                        format!("expression names '{name}', which is not an owner"),
                        "use self, target, here, world, place:<id>, story:<id>, or a cast id",
                    ));
                }
            }
        }
    }
}

fn push_rule_exprs<'a>(rule: &'a Rule, exprs: &mut Vec<&'a Expr>) {
    if let Some(roll) = &rule.roll {
        exprs.push(&roll.threshold);
        if let Some(band) = &roll.partial_at {
            exprs.push(band);
        }
    }
    for cost in rule.costs() {
        push_cost(cost, exprs);
    }
    for cond in &rule.if_ {
        cond.visit(|cond| {
            if let Cond::Meter { value, .. } = cond {
                exprs.push(value);
            }
        });
    }
    rule.visit_templates(|effect| {
        if let EffectTemplate::Modifier { add, .. } = effect {
            exprs.push(add);
        }
        if let EffectTemplate::Meter { add, set, .. } = effect {
            if let Some(expr) = add {
                exprs.push(expr);
            }
            if let Some(expr) = set {
                exprs.push(expr);
            }
        }
        if let Some(ResistSpec::Roll { threshold, .. }) = effect.resist() {
            exprs.push(threshold);
        }
    });
}

fn push_cost<'a>(cost: &'a Cost, exprs: &mut Vec<&'a Expr>) {
    for (_, expr) in cost.meters.iter().chain(cost.items.iter()) {
        exprs.push(expr);
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

/// Check the save against the pack without changing it. Refuse every component on an actor
/// with no kind, every actor without a place or an action state, every actor in a place or on
/// an exit the pack lacks, every meter, item and wielded slot the pack does not define, every
/// meter the pack gives an entity that the entity lacks, every meter outside its rules, every
/// modifier the pack no longer gives that meter, and every empty id or zero count.
pub fn check_save(canon: &Canon, state: &crate::state::State) -> Result<(), Vec<Diagnostic>> {
    const FILE: &str = "state/save.yaml";
    let mut diags = Vec::new();
    let actors = &state.actors();
    // Every store, with no `..`: a store added later must be listed here to compile.
    let state::entity::Actors {
        kind: _,
        place,
        wielded,
        known_facts,
        visited,
        acts,
        items,
        meters,
        modifiers,
        flags,
    } = actors;
    let held: BTreeSet<(&str, &ActorId)> = (place.keys().map(|id| ("place", id)))
        .chain(wielded.keys().map(|id| ("wielded", id)))
        .chain(known_facts.keys().map(|id| ("known_facts", id)))
        .chain(visited.keys().map(|(id, _)| ("visited", id)))
        .chain(acts.keys().map(|id| ("acts", id)))
        .chain(items.keys().map(|(id, _)| ("items", id)))
        .chain(meters.keys().map(|(id, _)| ("meters", id)))
        .chain(modifiers.keys().map(|(id, _)| ("modifiers", id)))
        .chain(flags.iter().map(|(id, _)| ("flags", id)))
        .collect();
    for (store, id) in held.iter().filter(|(_, id)| !actors.kind.contains_key(*id)) {
        let problem = format!("actor '{id}' has {store} but no kind");
        let diag = schema_error(FILE, &problem, "give the actor a kind under actors.kind");
        diags.push(diag.at_path(format!("actors.{store}.{id}")));
    }
    for id in actors.kind.keys() {
        for store in ["place", "acts"] {
            if !held.contains(&(store, id)) {
                let problem = format!("actor '{id}' has a kind but no {store}");
                let diag = schema_error(FILE, &problem, "give every actor a place and acts");
                diags.push(diag.at_path(format!("actors.{store}.{id}")));
            }
        }
    }
    for (id, place) in actors.place.iter() {
        // A walker stands on an exit, which must be one of the pack's.
        let known = match crate::state::exit_parts(place) {
            Some(_) => crate::rules::travel::exit_stood_on(canon, place).is_some(),
            None => canon.places.contains_key(place.as_str()),
        };
        if !known {
            let problem = format!("actor '{id}' is at '{place}', which the pack does not define");
            let diag = schema_error(FILE, &problem, "move the actor to a place");
            diags.push(diag.at_path(format!("actors.place.{id}")));
        }
    }
    for id in actors.kind.keys() {
        let held = nested_map::entries_of(&actors.meters, id).collect();
        let path = format!("actors.meters.{id}");
        check_meters(
            canon,
            &entity::Id::Actor(id.clone()),
            &held,
            &path,
            &mut diags,
        );
    }
    for ((id, key), live) in actors.modifiers.iter() {
        if !modifier::is_defined(&canon.modifiers, key, live) {
            let problem = format!(
                "actor '{id}' holds modifier '{key}' on meter '{}', which the pack does not define",
                live.meter
            );
            let hint = "run spook migrate to drop the modifier";
            let diag = schema_error(FILE, &problem, hint);
            diags.push(diag.at_path(format!("actors.modifiers.{id}.{key}")));
        }
    }
    let world_meters = state.globals().meters.iter();
    let world = world_meters.map(|(id, held)| (id.as_str(), held)).collect();
    check_meters(
        canon,
        &entity::Id::Global,
        &world,
        "world.meters",
        &mut diags,
    );
    let places = canon.places.keys().map(|id| {
        let held = nested_map::entries_of(&state.places().meters, id).collect();
        (entity::Id::Place(id.clone()), "places", id.as_str(), held)
    });
    let stories = canon.stories.keys().map(|id| {
        let held = nested_map::entries_of(&state.stories().meters, id).collect();
        (entity::Id::Story(id.clone()), "stories", id.as_str(), held)
    });
    for (carrier, list, id, held) in places.chain(stories) {
        let path = format!("{list}.meters.{id}");
        check_meters(canon, &carrier, &held, &path, &mut diags);
    }
    for ((id, item), count) in actors.items.iter() {
        if item.is_empty() {
            diags.push(
                schema_error(
                    FILE,
                    &format!("actor '{id}' lists an empty item id"),
                    "give the item a non-empty id",
                )
                .at_path(format!("actors.items.{id}")),
            );
        }
        if *count == 0 {
            diags.push(
                schema_error(
                    FILE,
                    &format!("actor '{id}' lists item '{item}' with a count of 0"),
                    "omit an item the actor does not hold",
                )
                .at_path(format!("actors.items.{id}.{item}")),
            );
        }
    }
    for ((place, item), count) in state.places().items.iter() {
        if *count == 0 {
            diags.push(
                schema_error(
                    FILE,
                    &format!("place '{place}' lists item '{item}' with a count of 0"),
                    "omit an item that is not there",
                )
                .at_path(format!("places.items.{place}.{item}")),
            );
        }
    }
    for (actor_id, item) in actors.items.keys() {
        if !canon.items.contains_key(item) {
            let path = format!("actors.items.{actor_id}.{item}");
            diags.push(unknown_item(FILE, item, &path));
        }
    }
    for (actor_id, wielded) in actors.wielded.iter() {
        if !canon.items.contains_key(wielded) {
            diags.push(unknown_item(
                FILE,
                wielded,
                &format!("actors.wielded.{actor_id}"),
            ));
        } else if !canon
            .items
            .get(wielded)
            .is_some_and(|item: &ItemDef| item.tags.iter().any(|tag| tag == "weapon"))
        {
            diags.push(
                schema_error(
                    FILE,
                    &format!("actor '{actor_id}' wields '{wielded}', which is not a weapon"),
                    "wield an item tagged weapon, or leave the slot empty",
                )
                .at_path(format!("actors.wielded.{actor_id}")),
            );
        } else if !actors
            .items
            .contains_key(&(actor_id.clone(), wielded.clone()))
        {
            diags.push(
                schema_error(
                    FILE,
                    &format!("actor '{actor_id}' wields '{wielded}' without holding it"),
                    "put the item in items or clear wielded",
                )
                .at_path(format!("actors.wielded.{actor_id}")),
            );
        }
    }
    for (place, item) in state.places().items.keys() {
        if !canon.items.contains_key(item) {
            let path = format!("places.items.{place}.{item}");
            diags.push(unknown_item(FILE, item, &path));
        }
    }
    if diags.is_empty() {
        Ok(())
    } else {
        Err(diags)
    }
}

/// The live meters of `carrier`, at `path` in the save, against the meters the pack gives it.
fn check_meters(
    canon: &Canon,
    carrier: &entity::Id,
    live: &BTreeMap<&str, &meter::Meter>,
    path: &str,
    diags: &mut Vec<Diagnostic>,
) {
    let (name, defs) = match carrier {
        entity::Id::Actor(id) => (format!("actor '{id}'"), &canon.meters),
        entity::Id::Global => ("the world".to_string(), &canon.world_meters),
        _ => (carrier.to_string(), &canon.meters),
    };
    for (key, def) in defs {
        let carried = *carrier == entity::Id::Global || def.is_on(carrier.as_ref());
        if carried && !live.contains_key(key.as_str()) {
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
        let Some(def) = canon.meter_def(carrier.as_ref(), key) else {
            diags.push(
                schema_error(
                    "state/save.yaml",
                    &format!("{name} lists meter '{key}', which the pack does not give it"),
                    "run spook migrate to drop the meter",
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
    diags.extend(check_cycles(canon));
    diags
}

/// `cycle` conditions, on a rule or an exit, that name no cycle in the budget or have a bound
/// at or past the cycle's length.
fn check_cycles(canon: &Canon) -> Vec<Diagnostic> {
    let rules = canon
        .rules
        .iter()
        .map(|rule| (&rule.file, format!("rules[{}].if", rule.index), &rule.if_));
    let exits = canon
        .places
        .values()
        .flat_map(|place| &place.exits)
        .map(|exit| {
            let path = format!("exits[{}].if", exit.index);
            (&exit.file, path, &exit.if_)
        });
    let mut diags = Vec::new();
    for (file, path, conds) in rules.chain(exits) {
        let mut cycles = Vec::new();
        for cond in conds {
            cond.visit(|cond| {
                if let Cond::Cycle { name, from, to } = cond {
                    cycles.push((name.as_str(), *from, *to));
                }
            });
        }
        for (name, from, to) in cycles {
            let Some(&length) = canon.budget.cycles.get(name) else {
                diags.push(
                    schema_error(
                        file,
                        &format!("there is no cycle {name}"),
                        "name a cycle from cycles in rules/budget.yaml, or add it there",
                    )
                    .at_path(path.clone()),
                );
                continue;
            };
            if from.max(to) >= length {
                diags.push(
                    schema_error(
                        file,
                        &format!("cycle bound is not below cycle {name}, which is {length} ticks"),
                        "set from and to to ticks of the cycle, below its length",
                    )
                    .at_path(path.clone()),
                );
            }
        }
    }
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
        if let Some(node) = canon.places.get(place.as_str()) {
            for exit in &node.exits {
                queue.push(exit.to.clone());
            }
        }
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
            rule.visit_templates(|effect| push_writes(canon, effect, &mut marks));
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

fn push_writes(canon: &Canon, effect: &EffectTemplate, out: &mut Vec<Write>) {
    match effect {
        EffectTemplate::Modifier { id, .. } => {
            if let Some(def) = canon.modifiers.get(id) {
                out.push(Write::Meter(def.meter.clone()));
            }
        }
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
    if watches_world(canon, watch) && !watches_actor_key(canon, watch) {
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

/// Whether `watch` can match a change no actor holds, which has no subject: a meter the world,
/// a place or a story carries, a flag a place or story declares, or a flag some rule sets with
/// `of: world`.
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
            let declared = |flags: &[String]| flags.contains(key);
            let doors = |place: &PlaceDef| place.exits.iter().any(|exit| !exit.if_.is_empty());
            (key == OPEN && canon.places.values().any(doors))
                || canon.places.values().any(|place| declared(&place.flags))
                || canon.stories.values().any(|story| declared(&story.flags))
                || canon.rules.iter().any(|rule| {
                    let mut sets = false;
                    rule.visit_templates(|effect| {
                        if let EffectTemplate::Flag {
                            of: grammar::Object::Global,
                            key: written,
                            ..
                        } = effect
                        {
                            sets |= written == key;
                        }
                    });
                    sets
                })
        }
        _ => false,
    }
}

/// Whether an actor may also hold the meter or flag `watch` names, so the watch can match an
/// actor's change as well as the world's.
fn watches_actor_key(canon: &Canon, watch: &ChangeWatch) -> bool {
    match watch {
        ChangeWatch::Meter { key, .. } => {
            canon.meters.get(key).is_some_and(MeterDef::is_actor_meter)
        }
        ChangeWatch::Flag { key, .. } => canon.cast.values().any(|cast| cast.flags.contains(key)),
        _ => false,
    }
}

/// A change rule on a world change runs once, for no one, so it has no `self` and no `here`.
/// Every other rule has a subject, so its `narrate` names who hears it.
fn check_world_change_subjects(canon: &Canon) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    for rule in canon.rules.iter() {
        let on_world = matches!(&rule.when, When::Change(watch) if watches_world(canon, watch));
        let (message, hint) = if on_world && names_subject(rule) {
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
        diags.push(
            schema_error(
                &rule.file,
                &format!("rule '{}' {message}", rule.local_id),
                hint,
            )
            .at_path(format!("rules[{}]", rule.index)),
        );
    }
    diags
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

/// Whether `rule` needs a subject: `self` or `here` as an `of`, a resist's `subject` or a
/// narrate's `to`, a location effect, which moves the subject, or a roll, which is the subject's.
fn names_subject(rule: &Rule) -> bool {
    let mut objects = Vec::new();
    for cond in &rule.if_ {
        cond.visit(|cond| match cond {
            Cond::HasItem { of, .. }
            | Cond::Meter { of, .. }
            | Cond::Flag { of, .. }
            | Cond::At { of, .. }
            | Cond::Knows { of, .. }
            | Cond::Wielding { of, .. } => objects.push(of),
            _ => {}
        });
    }
    let mut exprs = Vec::new();
    push_rule_exprs(rule, &mut exprs);
    for expr in exprs {
        objects.extend(expr.meter_reads().into_iter().map(|(of, _, _)| of));
    }
    let mut needs = rule.roll.is_some();
    rule.visit_templates(|effect| {
        match effect {
            EffectTemplate::Meter { of, .. }
            | EffectTemplate::Flag { of, .. }
            | EffectTemplate::Modifier { of, .. }
            | EffectTemplate::Memory { of, .. }
            | EffectTemplate::Item { of, .. }
            | EffectTemplate::Reveal { of, .. }
            | EffectTemplate::Wield { of, .. }
            | EffectTemplate::Unwield { of } => objects.push(of),
            EffectTemplate::Narrate { to, .. } => {
                needs |= matches!(to, NarrateTo::Self_ | NarrateTo::Here);
            }
            EffectTemplate::Location { .. } => needs = true,
            EffectTemplate::Note { .. } => {}
        }
        if let Some(ResistSpec::Roll { subject, .. }) = effect.resist() {
            objects.push(subject);
        }
    });
    needs
        || objects
            .iter()
            .any(|of| matches!(of, grammar::Object::Self_ | grammar::Object::Here))
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
