//! Turn a rule template into effects for one intent.

use crate::canon::{Canon, OPEN};
use crate::ecs::component::meter;
use crate::ecs::id::ActorId;
use crate::ecs::{self, component};
use crate::entity;
use crate::intent::Target;
use crate::round::dice::Roller;
use crate::rules::context::Context;
use crate::rules::effect::{Effect, EffectKind};
use crate::rules::expr::{self, Dice, Expr};
use crate::rules::grammar;
use crate::rules::roll::Outcome;
use crate::rules::roll::{MoveReport, Roll};
use crate::rules::rule::{EffectTemplate, LocationTo, NarrateTo, ResistSpec, Rule};
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

/// What a rule's effects came to: the effects, the roll that picked them, the resist rolls
/// made on the way, and why.
pub(crate) struct Expansion {
    pub effects: Vec<Effect>,
    pub rolled: Option<Roll>,
    pub resisted: Vec<MoveReport>,
    pub cause: Cause,
}

/// Roll the rule's `roll:` spec unless `rolled` already holds its roll, then expand the lists
/// its band picks, with the resist rolls `roller` makes on the way. More effects than
/// `max_effects_per_intent` is `BUDGET`.
pub(crate) fn expand(
    context: &Context,
    rule: &Rule,
    cause: Cause,
    rolled: Option<Roll>,
    mut roller: Roller,
) -> Result<Expansion, Reason> {
    let rolled = match (rolled, &rule.roll) {
        (None, Some(spec)) => Some(roller.roll_spec(context, spec)?),
        (rolled, _) => rolled,
    };
    let outcome = rolled.as_ref().map(|out| out.outcome);
    let effects = build_effects(context, rule, outcome, &mut roller)?;
    if effects.len() > context.canon.budget.max_effects_per_intent as usize {
        return Err(Reason::Budget);
    }
    Ok(Expansion {
        effects,
        rolled,
        resisted: roller.into_reports(),
        cause,
    })
}

/// The effects the rule's `do` list makes, with faces of 1 and no resist: their shapes are
/// what the pick stage and habits read before the round rolls.
pub(crate) fn preview(context: &Context, rule: &Rule) -> Result<Vec<Effect>, Reason> {
    build_effects(context, rule, None, &mut PreviewDice)
}

/// `do`, then the list `outcome` picks: `success`, `partial` or `miss`. `None` means the
/// rule did not roll. `dice.resist` returns the subject's band; success shrugs the effect off.
fn build_effects(
    context: &Context,
    rule: &Rule,
    outcome: Option<Outcome>,
    dice: &mut dyn Dice,
) -> Result<Vec<Effect>, Reason> {
    let mut selected = Vec::new();
    selected.extend(rule.do_.iter());
    match outcome {
        Some(Outcome::Success) => selected.extend(rule.success.iter()),
        Some(Outcome::Partial) => selected.extend(rule.partial.iter()),
        Some(Outcome::Miss) => selected.extend(rule.miss.iter()),
        None => {}
    }
    let mut effects = Vec::with_capacity(selected.len());
    let mut last_resist = None;
    for template in selected {
        push_effects(context, template, &mut effects, &mut last_resist, dice)?;
    }
    Ok(effects)
}

fn push_effects(
    context: &Context,
    template: &EffectTemplate,
    effects: &mut Vec<Effect>,
    last_resist: &mut Option<Outcome>,
    dice: &mut dyn Dice,
) -> Result<(), Reason> {
    let (canon, world, intent) = (context.canon, context.world, context.intent);
    let viewer = intent.actor.as_ref();
    if let Some(spec) = template.resist() {
        let shrugged = match spec {
            ResistSpec::Same => match last_resist {
                Some(Outcome::Success) => true,
                Some(_) => false,
                // No earlier resist roll: the loader refuses this (`resist_same_without_roll`).
                None => return Err(Reason::Schema),
            },
            ResistSpec::Roll {
                subject,
                check,
                threshold,
                resisted,
            } => {
                let actor = expr::resolve_actor(context, subject)?;
                let number = expr::eval_with_dice(context, threshold, dice)?;
                let band = dice.resist(canon, world, actor, check, number)?;
                *last_resist = Some(band);
                if band == Outcome::Success {
                    for alt in resisted {
                        push_effects(context, alt, effects, last_resist, dice)?;
                    }
                    return Ok(());
                }
                false
            }
        };
        if shrugged {
            return Ok(());
        }
    }
    match template {
        EffectTemplate::Flag { of, key, value, .. } => {
            let kind = match of {
                grammar::Object::Exit(exit) if key == OPEN => EffectKind::ExitOpen {
                    exit: exit.clone(),
                    open: *value,
                },
                _ => EffectKind::Flag {
                    entity: expr::resolve_entity(context, of)?,
                    key: key.clone(),
                    value: *value,
                },
            };
            effects.push(Effect::new(viewer, &intent.id, kind));
        }
        EffectTemplate::Modifier { to, id, add, ticks } => {
            effects.push(Effect::new(
                viewer,
                &intent.id,
                EffectKind::Modifier {
                    actor_id: expr::resolve_actor(context, to)?.clone(),
                    id: id.clone(),
                    add: expr::eval_with_dice(context, add, dice)?,
                    ticks: *ticks,
                },
            ));
        }
        EffectTemplate::Location { to, teleport } => {
            let place = match to {
                LocationTo::Via(via) => {
                    let exit = crate::rules::travel::exit_via(context, via)?;
                    exit.ok_or(Reason::Schema)?.to.clone()
                }
                LocationTo::Place(id) if *teleport => id.clone(),
                LocationTo::Target if *teleport => match &intent.target {
                    Target::Place(id) => id.to_string(),
                    _ => return Err(Reason::Schema),
                },
                LocationTo::Place(_) | LocationTo::Target => return Err(Reason::NotInSpec),
            };
            effects.push(location_effect(context, &place)?);
        }
        EffectTemplate::Meter {
            of, key, add, set, ..
        } => {
            let entity = expr::resolve_entity(context, of)?;
            let write = meter_offset(context, &entity, key, add.as_ref(), set.as_ref(), dice)?;
            effects.push(Effect::new(
                viewer,
                &intent.id,
                EffectKind::Meter {
                    entity,
                    key: key.clone(),
                    write,
                },
            ));
        }
        EffectTemplate::Item {
            of, id, add, max, ..
        } => {
            let item_id = expr::resolve_item_id(context, id)?;
            let (actor_id, place) = match expr::resolve_entity(context, of)? {
                entity::Id::Actor(id) => (Some(id), None),
                entity::Id::Place(id) => (None, Some(id)),
                entity::Id::Story(_) | entity::Id::Global => return Err(Reason::Schema),
            };
            effects.push(Effect::new(
                viewer,
                &intent.id,
                EffectKind::Item {
                    actor_id,
                    place,
                    id: item_id,
                    delta: *add,
                    max: *max,
                },
            ));
        }
        EffectTemplate::Wield { subject, id } => {
            let actor_id = expr::resolve_actor(context, subject)?.clone();
            effects.push(Effect::new(
                viewer,
                &intent.id,
                EffectKind::Wield {
                    actor_id,
                    id: expr::resolve_item_id(context, id)?,
                },
            ));
        }
        EffectTemplate::Unwield { subject } => {
            let actor_id = expr::resolve_actor(context, subject)?.clone();
            effects.push(Effect::new(
                viewer,
                &intent.id,
                EffectKind::Unwield { actor_id },
            ));
        }
        EffectTemplate::Note {
            id,
            to_ids,
            place_id,
            fact_ids,
            text,
        } => {
            effects.push(Effect::new(
                viewer,
                &intent.id,
                EffectKind::Note {
                    note_id: id.clone(),
                    to_ids: to_ids.clone(),
                    place_id: place_id.clone(),
                    fact_ids: fact_ids.clone(),
                    text: text.clone(),
                },
            ));
        }
        EffectTemplate::Narrate {
            to, text, fact_ids, ..
        } => {
            let to = match to {
                NarrateTo::Here => world.actors_at(expr::resolve_here(context)?),
                NarrateTo::Self_ => {
                    vec![expr::resolve_actor(context, &grammar::Object::Self_)?.clone()]
                }
                NarrateTo::Actors(ids) => ids.clone(),
                NarrateTo::Present => context.presence.actors_present.clone(),
            };
            effects.push(Effect::new(
                viewer,
                &intent.id,
                EffectKind::Narrate {
                    to,
                    fact_ids: fact_ids.clone(),
                    text: text.clone(),
                },
            ));
        }
        EffectTemplate::Memory { to, fact_ids, text } => {
            let actor_id = expr::resolve_actor(context, to)?.clone();
            effects.push(Effect::new(
                viewer,
                &intent.id,
                EffectKind::Memory {
                    actor_id,
                    fact_ids: fact_ids.clone(),
                    tags: Vec::new(),
                    text: text.clone(),
                },
            ));
        }
        EffectTemplate::Reveal { to, fact } => {
            let actor_id = expr::resolve_actor(context, to)?.clone();
            effects.push(Effect::new(
                viewer,
                &intent.id,
                EffectKind::Reveal {
                    actor_id,
                    fact: fact.clone(),
                },
            ));
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

fn location_effect(context: &Context, to: &str) -> Result<Effect, Reason> {
    let intent = context.intent;
    let actor_id = expr::resolve_actor(context, &grammar::Object::Self_)?.clone();
    let from = context
        .world
        .actors()
        .get::<component::Place>(&actor_id)
        .get();
    let from = from.ok_or(Reason::UnknownId)?.to_string();
    Ok(Effect::new(
        intent.actor.as_ref(),
        &intent.id,
        EffectKind::Location {
            actor_id,
            from,
            to: to.to_string(),
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

    /// A resist roll that does not shrug the effect off.
    fn resist(
        &mut self,
        _: &Canon,
        _: &ecs::World,
        _: &ActorId,
        _: &str,
        _: i64,
    ) -> Result<Outcome, Reason> {
        Ok(Outcome::Miss)
    }
}
