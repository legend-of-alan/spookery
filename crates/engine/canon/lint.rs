//! Cross-file validation of rules, exits, meters, items, facts, and triggers.

use std::collections::{BTreeMap, BTreeSet};

use crate::canon::source::{FileText, Locator, Problem};
use crate::canon::{declared_flags, Canon, CastDef, FactDef, ItemDef, PlaceDef, StoryDef, OPEN};
use crate::diagnostic::{error, schema_error, warning, Diagnostic};
use crate::ecs::component::meter;
use crate::ecs::component::modifier;
use crate::ecs::id::{ActorId, PlaceId, StoryId};
use crate::entity;
use crate::rules::cond;
use crate::rules::expr::Expr;
use crate::rules::grammar;
use crate::rules::rule::{
    self, ChangeWatch, Cond, EffectTemplate, LocationTo, NarrateTo, Owner, Rule, Slot, When,
};
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
                            exit.id, exit.to
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
                        &format!("hidden exit '{}' has no revealed_by fact", exit.id),
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
                            &format!("exit '{}' reveals '{fact}', which is not a fact", exit.id),
                            "use a fact id from this pack",
                        )
                        .at_path(format!("exits[{}].revealed_by", exit.index)),
                    );
                }
            }
            for id in collect_item_ids(exit.slots()) {
                if !items.contains_key(id) {
                    diags.push(
                        schema_error(
                            &exit.file,
                            &format!("exit '{}' names unknown item '{id}'", exit.id),
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
    rules: &rule::Table,
    items: &BTreeMap<String, ItemDef>,
) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    for rule in rules.iter() {
        for id in collect_item_ids(rule.slots()) {
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

/// The item ids `slots` name, each once and in order.
fn collect_item_ids<'l>(slots: Vec<(&'static str, Slot<'l>)>) -> BTreeSet<&'l str> {
    let ids = slots.into_iter().filter_map(|(_, slot)| match slot {
        Slot::Item(id) => Some(id),
        Slot::Owner(_) | Slot::Target(_) => None,
    });
    ids.collect()
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

pub(super) fn unknown_reads(scope: &EntityScope, rules: &rule::Table) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    for rule in rules.iter() {
        let exprs = rule.exprs();
        let path = format!("rules[{}]", rule.index);
        let source = FileText::named(&rule.file);
        scope.push_unknown_reads(&source, &path, fixed_self(rule), &exprs, &mut diags);
    }
    for place in scope.places.values() {
        for exit in &place.exits {
            let exprs = exit.exprs();
            let path = format!("exits[{}]", exit.index);
            let source = FileText::named(&exit.file);
            scope.push_unknown_reads(&source, &path, None, &exprs, &mut diags);
        }
    }
    diags
}

/// Meter and flag effects and conditions whose entity is fixed by the pack and does not carry
/// the meter or declare the flag. The world's flags are free names, so they are not checked.
pub(super) fn unknown_carriers(scope: &EntityScope, rules: &rule::Table) -> Vec<Diagnostic> {
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
    pub meters: &'a BTreeMap<String, meter::Descriptor>,
    pub world_meters: &'a BTreeMap<String, meter::Descriptor>,
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
            grammar::Object::Actor(grammar::Actor::Id(id)) if self.cast.contains_key(id) => {
                Some(entity::Id::Actor(id.clone()))
            }
            grammar::Object::Actor(grammar::Actor::Self_) => {
                fixed_self.map(|id| entity::Id::Actor(id.into()))
            }
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
        let desc = self.meters.get(key);
        if let grammar::Object::Exit(exit) = of {
            return Some(format!("{exit} carries no meter '{key}'"));
        }
        match self.fixed_entity(fixed_self, of) {
            Some(entity::Id::Global) if self.world_meters.contains_key(key) => None,
            Some(entity::Id::Global) => Some(format!("the world carries no meter '{key}'")),
            Some(id) if !desc.is_some_and(|desc| desc.is_on(id.as_ref())) => {
                Some(match desc.and_then(|desc| desc.carrier.as_ref()) {
                    Some(carrier) => format!("only {carrier} carries meter '{key}'"),
                    None => format!("{id} carries no meter '{key}'"),
                })
            }
            None if desc.is_none() => Some(format!("no file declares meter '{key}'")),
            _ => None,
        }
    }

    /// The id `of` gives when no place, story or cast member has it.
    fn unknown_object(&self, of: &grammar::Object) -> Option<String> {
        match of {
            grammar::Object::Actor(grammar::Actor::Self_ | grammar::Actor::Target)
            | grammar::Object::Here
            | grammar::Object::Global => None,
            grammar::Object::Place(id) if self.places.contains_key(id) => None,
            grammar::Object::Place(id) => Some(format!("place:{id}")),
            grammar::Object::Story(id) if self.stories.contains_key(id) => None,
            grammar::Object::Story(id) => Some(format!("story:{id}")),
            grammar::Object::Exit(exit) => {
                let exits = self.places.get(&exit.place).map(|at| at.exits.as_slice());
                let known = exits.is_some_and(|exits| exits.iter().any(|at| at.id == exit.exit));
                (!known).then(|| exit.to_string())
            }
            grammar::Object::Actor(grammar::Actor::Id(id)) if self.cast.contains_key(id) => None,
            grammar::Object::Actor(grammar::Actor::Id(id)) => Some(id.to_string()),
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
                if let Some(id) = self.unknown_object(of) {
                    diags.push(refuse(
                        format!("expression names '{id}', which is not an owner"),
                        "use self, target, here, world, place:<id>, story:<id>, or a cast id",
                    ));
                }
            }
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
    diags.extend(check_gm_notes_length(canon));
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
                    .at_path(path.clone()),
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
            if let Some(desc) = canon.modifiers.get(id) {
                out.push(Write::Meter(desc.meter.clone()));
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
            let constant_sign = add.constant().map(i64::signum);
            for (gained, sign) in [(true, 1), (false, -1)] {
                if constant_sign.is_none_or(|constant| constant == sign) {
                    out.push(Write::Item {
                        id: id.clone(),
                        gained,
                    });
                }
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
                    .is_some_and(|desc| !desc.is_actor_meter())
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
        ChangeWatch::Meter { key, .. } => canon
            .meters
            .get(key)
            .is_some_and(meter::Descriptor::is_actor_meter),
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

/// Whether `rule` needs a subject: `self` or `here` named as `of`, `subject` or `to`, a
/// nested check's `by`, a narrate's `to`, a location effect, which moves the subject, or a
/// check, which is the subject's.
fn names_subject(rule: &Rule) -> bool {
    let mut needs = rule.check.is_some();
    rule.visit_templates(|effect| match effect {
        EffectTemplate::Narrate { to, .. } => {
            needs |= matches!(to, NarrateTo::Self_ | NarrateTo::Here);
        }
        EffectTemplate::Location { .. } => needs = true,
        _ => {}
    });
    let self_slot = Slot::Owner(grammar::Object::Actor(grammar::Actor::Self_));
    let here = Slot::Owner(grammar::Object::Here);
    let names_self_or_here = |(_, slot): &(&str, Slot)| *slot == self_slot || *slot == here;
    needs || rule.slots().iter().any(names_self_or_here)
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
