//! The pack's fixed entities, and the rule and path reads of meters and flags they cannot carry.

use std::collections::BTreeMap;

use crate::canon::refusal::{FileText, Locator, Problem};
use crate::canon::{self, cast, map, place, story, OPEN};
use crate::diagnostic::{schema_error, Diagnostic};
use crate::ecs::component::meter;
use crate::ecs::id::{ActorId, PlaceId, StoryId};
use crate::entity;
use crate::rules::expr::Expr;
use crate::rules::grammar;
use crate::rules::rule::{self, Cond, EffectTemplate, Owner, Rule, TargetKind, When};

pub(in crate::canon) fn find_unknown_meter_reads(
    scope: &EntityDefs,
    rules: &rule::Table,
) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    for rule in rules.iter() {
        let exprs = rule.exprs();
        let at = format!("rules[{}]", rule.index);
        let source = FileText::without_text(&rule.file);
        scope.push_unknown_reads(&source, &at, fixed_self_id(rule), &exprs, &mut diags);
    }
    for (row, path) in scope.paths.iter().enumerate() {
        let at = format!("paths[{row}]");
        let source = FileText::without_text(map::FILE);
        scope.push_unknown_reads(&source, &at, None, &path.exprs(), &mut diags);
    }
    diags
}

/// Meter effects and conditions whose entity is fixed by the pack and does not carry the meter,
/// and flag effects and conditions on a path that cannot carry the flag. Any other entity may
/// carry any flag.
pub(in crate::canon) fn find_missing_components(
    scope: &EntityDefs,
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
        let fixed = fixed_self_id(rule);
        let target = rule.when.target_kind();
        for (kind, of, key) in named {
            let why = match kind {
                "meter" => scope.meter_not_carried_reason(fixed, of, key),
                _ => scope.path_lacks_flag_reason(of, key, target),
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
fn fixed_self_id(rule: &Rule) -> Option<&str> {
    match (&rule.owner, &rule.when) {
        (Owner::Cast(id), When::Habit) => Some(id),
        _ => None,
    }
}

/// The meters each entity carries, and the cast, places, exits and stories an `of` may name.
pub(in crate::canon) struct EntityDefs<'c> {
    pub meters: &'c BTreeMap<String, meter::Def>,
    pub world_meters: &'c BTreeMap<String, meter::Def>,
    pub cast: &'c BTreeMap<ActorId, cast::Def>,
    pub places: &'c BTreeMap<PlaceId, place::Def>,
    pub paths: &'c canon::Paths,
    pub stories: &'c BTreeMap<StoryId, story::Def>,
}

impl EntityDefs<'_> {
    /// The one entity `of` names whatever the round: `world`, a place, a story, a cast id, or
    /// `self` on a cast member's own habit. `None` when only play time knows it.
    fn fixed_entity_id(
        &self,
        fixed_self_id: Option<&str>,
        of: &grammar::Object,
    ) -> Option<entity::Id> {
        match of {
            grammar::Object::Global => Some(entity::Id::Global),
            grammar::Object::Place(id) => Some(entity::Id::Place(id.clone())),
            grammar::Object::Story(id) => Some(entity::Id::Story(id.clone())),
            grammar::Object::Actor(grammar::Actor::Id(id)) if self.cast.contains_key(id) => {
                Some(entity::Id::Actor(id.clone()))
            }
            grammar::Object::Actor(grammar::Actor::Self_) => {
                fixed_self_id.map(|id| entity::Id::Actor(id.into()))
            }
            _ => None,
        }
    }

    /// Why flag `key` of the path `of` names can never be set or tested, or `None` when it may be
    /// or `of` names no path. `target` is what the rule's move targets, so `of: target` names a
    /// path only on a move that targets one.
    fn path_lacks_flag_reason(
        &self,
        of: &grammar::Object,
        key: &str,
        target: TargetKind,
    ) -> Option<String> {
        let path = match of {
            grammar::Object::Path(id) => match self.paths.get(id) {
                Some(path) => Some(path),
                None => return Some(format!("path:{id} is not a path")),
            },
            grammar::Object::Actor(grammar::Actor::Target) if target == TargetKind::Path => None,
            _ => return None,
        };
        if key != OPEN {
            return Some(format!("a path carries only the flag '{OPEN}'"));
        }
        let always_open_path = path.filter(|path| !path.is_conditional());
        always_open_path.map(|path| format!("path:{} has no if, so it is always open", path.id))
    }

    /// Why meter `key` of the owner `of` names can never be read or written, or `None` when it
    /// may be.
    /// An entity known only at play time is not guessed: only its key is checked.
    fn meter_not_carried_reason(
        &self,
        fixed_self_id: Option<&str>,
        of: &grammar::Object,
        key: &str,
    ) -> Option<String> {
        let desc = self.meters.get(key);
        if let grammar::Object::Path(id) = of {
            return Some(format!("path:{id} carries no meter '{key}'"));
        }
        match self.fixed_entity_id(fixed_self_id, of) {
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

    /// The id `of` gives when no place, path, story or cast member has it.
    fn unknown_object_id(&self, of: &grammar::Object) -> Option<String> {
        match of {
            grammar::Object::Actor(grammar::Actor::Self_ | grammar::Actor::Target)
            | grammar::Object::Here
            | grammar::Object::Global => None,
            grammar::Object::Place(id) if self.places.contains_key(id) => None,
            grammar::Object::Place(id) => Some(format!("place:{id}")),
            grammar::Object::Story(id) if self.stories.contains_key(id) => None,
            grammar::Object::Story(id) => Some(format!("story:{id}")),
            grammar::Object::Path(id) if self.paths.get(id).is_some() => None,
            grammar::Object::Path(id) => Some(format!("path:{id}")),
            grammar::Object::Actor(grammar::Actor::Id(id)) if self.cast.contains_key(id) => None,
            grammar::Object::Actor(grammar::Actor::Id(id)) => Some(id.to_string()),
        }
    }

    /// The unknown reads in `exprs`, at `path` in `source` and at each read's spot.
    fn push_unknown_reads(
        &self,
        source: &FileText,
        path: &str,
        fixed_self_id: Option<&str>,
        exprs: &[&Expr],
        diags: &mut Vec<Diagnostic>,
    ) {
        for expr in exprs {
            for read in expr.reads() {
                let Expr::Meter { of, key, spot } = read else {
                    continue;
                };
                let at = Locator::FileLocation(*spot);
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
                if let Some(why) = self.meter_not_carried_reason(fixed_self_id, of, key) {
                    diags.push(refuse(
                        format!("expression reads meter '{key}', but {why}"),
                        "declare the meter on that entity, or read one that exists",
                    ));
                }
                if let Some(id) = self.unknown_object_id(of) {
                    diags.push(refuse(
                        format!("expression names '{id}', which is not an owner"),
                        "use self, target, here, world, place:<id>, story:<id>, or a cast id",
                    ));
                }
            }
        }
    }
}
