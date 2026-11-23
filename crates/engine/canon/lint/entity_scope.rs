//! The pack's fixed entities, and the rule and path reads of meters and flags they cannot carry.

use std::collections::BTreeMap;

use crate::canon::map::MAP_FILE;
use crate::canon::source::{FileText, Locator, Problem};
use crate::canon::{CastDef, Exit, Paths, PlaceDef, StoryDef, OPEN};
use crate::diagnostic::{schema_error, Diagnostic};
use crate::ecs::component::meter;
use crate::ecs::id::{ActorId, ExitId, PlaceId, StoryId};
use crate::entity;
use crate::rules::expr::Expr;
use crate::rules::grammar;
use crate::rules::rule::{self, Cond, EffectTemplate, Owner, Rule, When};

pub(in crate::canon) fn unknown_reads(scope: &EntityScope, rules: &rule::Table) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    for rule in rules.iter() {
        let exprs = rule.exprs();
        let path = format!("rules[{}]", rule.index);
        let source = FileText::named(&rule.file);
        scope.push_unknown_reads(&source, &path, fixed_self(rule), &exprs, &mut diags);
    }
    for (row, path) in scope.paths.iter().enumerate() {
        let at = format!("paths[{row}]");
        let source = FileText::named(MAP_FILE);
        scope.push_unknown_reads(&source, &at, None, &path.exprs(), &mut diags);
    }
    diags
}

/// Meter effects and conditions whose entity is fixed by the pack and does not carry the meter,
/// and flag effects and conditions on an exit that cannot carry the flag. Any other entity may
/// carry any flag.
pub(in crate::canon) fn unknown_carriers(
    scope: &EntityScope,
    rules: &rule::Table,
) -> Vec<Diagnostic> {
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
                _ => scope.why_exit_lacks_flag(of, key),
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

/// The meters each entity carries, and the cast, places, exits and stories an `of` may name.
pub(in crate::canon) struct EntityScope<'a> {
    pub meters: &'a BTreeMap<String, meter::Descriptor>,
    pub world_meters: &'a BTreeMap<String, meter::Descriptor>,
    pub cast: &'a BTreeMap<ActorId, CastDef>,
    pub places: &'a BTreeMap<PlaceId, PlaceDef>,
    pub paths: &'a Paths,
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

    /// Why flag `key` of the exit `of` names can never be set or tested, or `None` when it may be
    /// or `of` is no exit.
    fn why_exit_lacks_flag(&self, of: &grammar::Object, key: &str) -> Option<String> {
        let grammar::Object::Exit(exit) = of else {
            return None;
        };
        if self.unknown_object(of).is_some() {
            Some(format!("{exit} is not an exit"))
        } else if key != OPEN {
            Some(format!("an exit carries only the flag '{OPEN}'"))
        } else {
            let is_conditional = self
                .find_exit(exit)
                .is_some_and(|exit| exit.path.is_conditional());
            (!is_conditional).then(|| format!("{exit} has no if, so it is always open"))
        }
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

    /// The exit `exit` names, when its place has one by that word.
    fn find_exit(&self, exit: &ExitId) -> Option<Exit<'_>> {
        self.paths.find_exit(exit.place.as_str(), &exit.exit)
    }

    /// The id `of` gives when no place, exit, story or cast member has it.
    fn unknown_object(&self, of: &grammar::Object) -> Option<String> {
        match of {
            grammar::Object::Actor(grammar::Actor::Self_ | grammar::Actor::Target)
            | grammar::Object::Here
            | grammar::Object::Global => None,
            grammar::Object::Place(id) if self.places.contains_key(id) => None,
            grammar::Object::Place(id) => Some(format!("place:{id}")),
            grammar::Object::Story(id) if self.stories.contains_key(id) => None,
            grammar::Object::Story(id) => Some(format!("story:{id}")),
            grammar::Object::Exit(exit) => self.find_exit(exit).is_none().then(|| exit.to_string()),
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
            for read in expr.reads() {
                let Expr::Meter { of, key, spot } = read else {
                    continue;
                };
                let at = Locator::Spot(*spot);
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
