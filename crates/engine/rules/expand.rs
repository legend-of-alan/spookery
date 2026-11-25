//! Turn a rule template into effects for one intent.

use crate::ecs::component;
use crate::ecs::component::meter;
use crate::ecs::component::ActorLocation;
use crate::entity;
use crate::intent::Target;
use crate::round::dice::Roller;
use crate::rules::context::Context;
use crate::rules::effect::{Effect, EffectKind};
use crate::rules::expr::{self, Dice, Expr};
use crate::rules::grammar;
use crate::rules::roll::Outcome;
use crate::rules::roll::{MoveReport, Roll};
use crate::rules::rule::{EffectTemplate, LocationTo, NarrateTo, Rule};
use crate::Reason;

/// Why a rule's effects are expanded and landed, which decides what the landing journals.
#[derive(Clone, Copy)]
pub(crate) enum Cause {
    /// An action finishing, now or when its ticks pass: each effect takes an `EffectApplied`
    /// step, and a refusal is told to a player.
    Finish,
    /// A change rule firing, which is nobody's move: its effects take no step of their own and
    /// its refusal is told to nobody.
    Trigger,
}

/// What a rule's effects came to: the effects, the roll that picked them, the nested checks'
/// rolls made on the way, each told to its `by` actor, and why.
pub(crate) struct Expansion {
    pub effects: Vec<Effect>,
    pub rolled: Option<Roll>,
    pub reports: Vec<MoveReport>,
    pub cause: Cause,
}

/// Roll the rule's `let:`, then its `check:`, then expand the lists its band picks, rolling
/// each nested check on the way, all on `roller`. More effects than `max_effects_per_intent` is
/// `BUDGET`.
pub(crate) fn expand(
    context: &Context,
    rule: &Rule,
    cause: Cause,
    mut roller: Roller,
) -> Result<Expansion, Reason> {
    let lets = expr::roll_lets(context, &rule.lets, &mut roller)?;
    let context = &context.bind(&lets);
    let rolled = match &rule.check {
        Some(check) => Some(check.roll(context, &mut roller)?),
        None => None,
    };
    let outcome = rolled.as_ref().map(|out| out.outcome);
    let mut reports = Vec::new();
    let effects = build_effects(context, rule, outcome, &mut roller, &mut reports)?;
    if effects.len() > context.canon.budget.max_effects_per_intent as usize {
        return Err(Reason::Budget);
    }
    Ok(Expansion {
        effects,
        rolled,
        reports,
        cause,
    })
}

/// The effects the rule's `do` list makes, with faces of 1 and its `let:` and nested checks
/// rolled on them, told to nobody: their shapes are what the pick stage and habits read before
/// the round rolls.
pub(crate) fn preview(context: &Context, rule: &Rule) -> Result<Vec<Effect>, Reason> {
    let lets = expr::roll_lets(context, &rule.lets, &mut PreviewDice)?;
    let context = &context.bind(&lets);
    build_effects(context, rule, None, &mut PreviewDice, &mut Vec::new())
}

/// `do`, then the list `outcome` picks: `success`, `partial` or `miss`. `None` means the
/// rule did not roll. Each nested check's roll goes on `reports`.
fn build_effects(
    context: &Context,
    rule: &Rule,
    outcome: Option<Outcome>,
    dice: &mut dyn Dice,
    reports: &mut Vec<MoveReport>,
) -> Result<Vec<Effect>, Reason> {
    let picked =
        outcome.map(|outcome| select_list(outcome, &rule.success, &rule.partial, &rule.miss));
    let mut effects = Vec::new();
    for template in rule.do_.iter().chain(picked.into_iter().flatten()) {
        push_effects(context, template, &mut effects, reports, dice)?;
    }
    Ok(effects)
}

/// The list `outcome` picks.
fn select_list<'c>(
    outcome: Outcome,
    success: &'c [EffectTemplate],
    partial: &'c [EffectTemplate],
    miss: &'c [EffectTemplate],
) -> &'c [EffectTemplate] {
    match outcome {
        Outcome::Success => success,
        Outcome::Partial => partial,
        Outcome::Miss => miss,
    }
}

fn push_effects(
    context: &Context,
    template: &EffectTemplate,
    effects: &mut Vec<Effect>,
    reports: &mut Vec<MoveReport>,
    dice: &mut dyn Dice,
) -> Result<(), Reason> {
    let (world, intent) = (context.world, context.intent);
    match template {
        EffectTemplate::Flag { of, key, value } => {
            let kind = EffectKind::Flag {
                entity: of.entity_id(context)?,
                key: key.clone(),
                value: *value,
            };
            effects.push(Effect::new(intent.actor_id.as_ref(), &intent.id, kind));
        }
        EffectTemplate::Modifier { to, id, add, ticks } => {
            effects.push(Effect::new(
                intent.actor_id.as_ref(),
                &intent.id,
                EffectKind::Modifier {
                    actor_id: to.actor_id(context)?.clone(),
                    id: id.clone(),
                    add: expr::eval_with_dice(context, add, dice)?,
                    ticks: (u64::try_from(expr::eval_with_dice(context, ticks, dice)?).ok())
                        .filter(|ticks| *ticks >= 1)
                        .ok_or(Reason::OutOfRange)?,
                },
            ));
        }
        EffectTemplate::Location { to, teleport } => {
            let place_id = match to {
                LocationTo::Via(via) => {
                    let exit = crate::rules::travel::exit_via(context, via)?;
                    exit.ok_or(Reason::Schema)?.to_place_id().clone()
                }
                LocationTo::Place(id) if *teleport => id.as_str().into(),
                LocationTo::Target if *teleport => match &intent.target {
                    Target::Place(id) => id.clone(),
                    _ => return Err(Reason::Schema),
                },
                LocationTo::Place(_) | LocationTo::Target => return Err(Reason::NotInSpec),
            };
            effects.push(location_effect(context, ActorLocation::Place(place_id))?);
        }
        EffectTemplate::Meter { of, key, add, set } => {
            let entity = of.entity_id(context)?;
            let write = meter_offset(context, &entity, key, add.as_ref(), set.as_ref(), dice)?;
            effects.push(Effect::new(
                intent.actor_id.as_ref(),
                &intent.id,
                EffectKind::Meter {
                    entity,
                    key: key.clone(),
                    write,
                },
            ));
        }
        EffectTemplate::Item { of, id, add, max } => {
            let item_id = context.item_id(id)?;
            let (actor_id, place) = match of.entity_id(context)? {
                entity::Id::Actor(id) => (Some(id), None),
                entity::Id::Place(id) => (None, Some(id)),
                entity::Id::Path(_) | entity::Id::Story(_) | entity::Id::Global => {
                    return Err(Reason::Schema)
                }
            };
            effects.push(Effect::new(
                intent.actor_id.as_ref(),
                &intent.id,
                EffectKind::Item {
                    actor_id,
                    place,
                    id: item_id,
                    delta: expr::eval_with_dice(context, add, dice)?,
                    max: (max.as_ref())
                        .map(|max| expr::eval_with_dice(context, max, dice))
                        .transpose()?,
                },
            ));
        }
        EffectTemplate::Wield { subject, id } => {
            let actor_id = subject.actor_id(context)?.clone();
            effects.push(Effect::new(
                intent.actor_id.as_ref(),
                &intent.id,
                EffectKind::Wield {
                    actor_id,
                    id: context.item_id(id)?,
                },
            ));
        }
        EffectTemplate::Unwield { subject } => {
            let actor_id = subject.actor_id(context)?.clone();
            effects.push(Effect::new(
                intent.actor_id.as_ref(),
                &intent.id,
                EffectKind::Unwield { actor_id },
            ));
        }
        EffectTemplate::Note {
            id,
            to_ids,
            place_id,
            text,
        } => {
            effects.push(Effect::new(
                intent.actor_id.as_ref(),
                &intent.id,
                EffectKind::Note {
                    note_id: id.clone(),
                    to_ids: to_ids.clone(),
                    place_id: place_id.clone(),
                    text: text.clone(),
                },
            ));
        }
        EffectTemplate::Narrate { to, text } => {
            let to = match to {
                NarrateTo::Here => world.actors_at(context.actor_place_id()?),
                NarrateTo::Self_ => {
                    vec![grammar::Actor::Self_.actor_id(context)?.clone()]
                }
                NarrateTo::Actors(ids) => ids.clone(),
                NarrateTo::Present => context.presence.actors_present.clone(),
            };
            effects.push(Effect::new(
                intent.actor_id.as_ref(),
                &intent.id,
                EffectKind::Narrate {
                    to,
                    text: text.clone(),
                },
            ));
        }
        EffectTemplate::Memory { to, text } => {
            let actor_id = to.actor_id(context)?.clone();
            effects.push(Effect::new(
                intent.actor_id.as_ref(),
                &intent.id,
                EffectKind::Memory {
                    actor_id,
                    tags: Vec::new(),
                    text: text.clone(),
                },
            ));
        }
        EffectTemplate::Check {
            by,
            check,
            success,
            partial,
            miss,
        } => {
            let actor_id = by.actor_id(context)?.clone();
            let roll = check.roll(context, dice)?;
            let picked = select_list(roll.outcome, success, partial, miss);
            reports.push(MoveReport::Rolled { actor_id, roll });
            for template in picked {
                push_effects(context, template, effects, reports, dice)?;
            }
        }
    }
    Ok(())
}

fn meter_offset(
    context: &Context,
    id: &entity::Id,
    key: &str,
    add: Option<&Expr>,
    set: Option<&Expr>,
    dice: &mut dyn Dice,
) -> Result<meter::Write, Reason> {
    if let Some(expr) = add {
        return Ok(meter::Write::Offset(expr::eval_with_dice(
            context, expr, dice,
        )?));
    }
    let desired = expr::eval_with_dice(context, set.ok_or(Reason::Schema)?, dice)?;
    let current = (context.world.entities().get::<component::Meter>(id)).value(key)?;
    Ok(meter::Write::Offset(
        desired.checked_sub(current).ok_or(Reason::OutOfRange)?,
    ))
}

fn location_effect(context: &Context, to: ActorLocation) -> Result<Effect, Reason> {
    let intent = context.intent;
    let actor_id = grammar::Actor::Self_.actor_id(context)?.clone();
    let from = context
        .world
        .actors()
        .get::<component::Place>(&actor_id)
        .get();
    let from = from.ok_or(Reason::UnknownId)?.clone();
    Ok(Effect::new(
        intent.actor_id.as_ref(),
        &intent.id,
        EffectKind::Location {
            actor_id,
            from,
            to,
            arrived_at: None,
        },
    ))
}

/// Faces of 1, and no journal line. A preview only needs the effect shape.
/// The round rolls the real faces when it applies the intent.
struct PreviewDice;

impl Dice for PreviewDice {
    fn begin(&mut self) {}

    fn draw(&mut self, count: u32, _sides: u32) -> Result<Vec<u32>, Reason> {
        Ok(vec![1; count as usize])
    }

    fn push_line(&mut self, _text: String) {}
}
