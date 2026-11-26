//! The pack's fixed entities, and the rule and path reads of meters and flags they cannot carry.

use std::collections::BTreeMap;

use crate::canon::map::{self, OPEN};
use crate::canon::refusal::{FileText, Locator};
use crate::canon::{cast, place, story};
use crate::diagnostic::{schema_error, Diagnostic};
use crate::ecs::component::meter;
use crate::ecs::id::{ActorId, PlaceId, StoryId};
use crate::entity;
use crate::rules::expr::Expr;
use crate::rules::grammar;
use crate::rules::rule::{self, Cond, EffectTemplate, Rule, RuleKind, Scope, TargetKind};

pub(in crate::canon) fn find_unknown_meter_reads(
    entity_defs: &EntityDefs,
    rules: &rule::Table,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for rule in rules.iter() {
        let exprs = rule.exprs();
        let key_path = format!("rules[{}]", rule.index);
        let file_text = FileText::without_text(&rule.file);
        entity_defs.push_unknown_reads(
            &file_text,
            &key_path,
            fixed_self_id(rule),
            &exprs,
            &mut diagnostics,
        );
    }
    for (index, path_def) in entity_defs.paths.iter().enumerate() {
        let key_path = format!("paths[{index}]");
        let file_text = FileText::without_text(map::FILE);
        entity_defs.push_unknown_reads(
            &file_text,
            &key_path,
            None,
            &path_def.exprs(),
            &mut diagnostics,
        );
    }
    diagnostics
}

/// Meter effects and conds whose entity is fixed by the pack and does not carry the meter, and
/// flag effects and conds on a path that cannot carry the flag. Any other entity may carry any
/// flag.
pub(in crate::canon) fn find_missing_components(
    entity_defs: &EntityDefs,
    rules: &rule::Table,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for rule in rules.iter() {
        let mut named = Vec::new();
        for cond in &rule.if_ {
            cond.visit(|cond| match cond {
                Cond::Meter { of, id, .. } => named.push(("meter", of, id.as_str())),
                Cond::Flag { of, id, .. } => named.push(("flag", of, id.as_str())),
                _ => {}
            });
        }
        rule.visit_templates(|effect| match effect {
            EffectTemplate::Meter { of, id, .. } => named.push(("meter", of, id.as_str())),
            EffectTemplate::Flag { of, id, .. } => named.push(("flag", of, id.as_str())),
            _ => {}
        });
        let fixed_self_id = fixed_self_id(rule);
        let target_kind = rule.kind.target_kind();
        for (component, of, id) in named {
            let reason = match component {
                "meter" => entity_defs.meter_not_carried_reason(fixed_self_id, of, id),
                _ => entity_defs.path_lacks_flag_reason(of, id, target_kind),
            };
            let Some(reason) = reason else {
                continue;
            };
            diagnostics.push(
                schema_error(
                    &rule.file,
                    &format!(
                        "rule '{}' names {component} '{id}', but {reason}",
                        rule.local_id
                    ),
                    "declare it on that entity, or name an entity that has it",
                )
                .at_path(format!("rules[{}]", rule.index)),
            );
        }
    }
    diagnostics
}

/// The actor `self` always is on `rule`: the cast member, on its own habit.
fn fixed_self_id(rule: &Rule) -> Option<&str> {
    match (&rule.scope, &rule.kind) {
        (Scope::Cast(id), RuleKind::Habit) => Some(id),
        _ => None,
    }
}

/// The meters each entity carries, and the cast, places, exits and stories an `of` may name.
pub(in crate::canon) struct EntityDefs<'c> {
    pub meters: &'c BTreeMap<String, meter::Def>,
    pub world_meters: &'c BTreeMap<String, meter::Def>,
    pub cast: &'c BTreeMap<ActorId, cast::Def>,
    pub places: &'c BTreeMap<PlaceId, place::Def>,
    pub paths: &'c map::Paths,
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

    /// The reason flag `flag_id` of the path `of` names can never be set or tested, or `None`
    /// when it may be or `of` names no path. `target` is what the rule's move targets, so
    /// `of: target` names a path only on a move that targets one.
    fn path_lacks_flag_reason(
        &self,
        of: &grammar::Object,
        flag_id: &str,
        target: TargetKind,
    ) -> Option<String> {
        let path_def = match of {
            grammar::Object::Path(id) => match self.paths.get(id) {
                Some(path_def) => Some(path_def),
                None => return Some(format!("path:{id} is not a path")),
            },
            grammar::Object::Actor(grammar::Actor::Target) if target == TargetKind::Path => None,
            _ => return None,
        };
        if flag_id != OPEN {
            return Some(format!("a path carries only the flag '{OPEN}'"));
        }
        let always_open_path = path_def.filter(|path_def| !path_def.is_conditional());
        always_open_path
            .map(|path_def| format!("path:{} has no if, so it is always open", path_def.id))
    }

    /// The reason meter `meter_id` of the entity `of` names can never be read or written, or
    /// `None` when it may be.
    /// An entity known only at play time is not guessed: only its meter id is checked.
    fn meter_not_carried_reason(
        &self,
        fixed_self_id: Option<&str>,
        of: &grammar::Object,
        meter_id: &str,
    ) -> Option<String> {
        let def = self.meters.get(meter_id);
        if let grammar::Object::Path(id) = of {
            return Some(format!("path:{id} carries no meter '{meter_id}'"));
        }
        match self.fixed_entity_id(fixed_self_id, of) {
            Some(entity::Id::Global) if self.world_meters.contains_key(meter_id) => None,
            Some(entity::Id::Global) => Some(format!("the world carries no meter '{meter_id}'")),
            Some(id) if !def.is_some_and(|def| def.is_on(id.as_ref())) => {
                Some(match def.and_then(|def| def.entity_id.as_ref()) {
                    Some(entity_id) => format!("only {entity_id} carries meter '{meter_id}'"),
                    None => format!("{id} carries no meter '{meter_id}'"),
                })
            }
            None if def.is_none() => Some(format!("no file declares meter '{meter_id}'")),
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

    /// The unknown reads in `exprs`, at `key_path` in `file_text` and at each read's file
    /// location.
    fn push_unknown_reads(
        &self,
        file_text: &FileText,
        key_path: &str,
        fixed_self_id: Option<&str>,
        exprs: &[&Expr],
        diagnostics: &mut Vec<Diagnostic>,
    ) {
        for expr in exprs {
            for read in expr.reads() {
                let Expr::Meter {
                    of,
                    id: meter_id,
                    file_location,
                } = read
                else {
                    continue;
                };
                let at = Locator::FileLocation(*file_location);
                let refuse = |message, hint| file_text.refuse_at(key_path, at, message, hint);
                if let Some(reason) = self.meter_not_carried_reason(fixed_self_id, of, meter_id) {
                    diagnostics.push(refuse(
                        format!("expression reads meter '{meter_id}', but {reason}"),
                        "declare the meter on that entity, or read one that exists",
                    ));
                }
                if let Some(id) = self.unknown_object_id(of) {
                    diagnostics.push(refuse(
                        format!("expression names '{id}', which is not an owner"),
                        "use self, target, here, world, place:<id>, story:<id>, or a cast id",
                    ));
                }
            }
        }
    }
}
