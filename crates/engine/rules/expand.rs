//! Turn a rule template into effects for one intent.

use crate::canon::Canon;
use crate::intent::Target;
use crate::model::meter::MeterWrite;
use crate::model::State;
use crate::round::dice::Roller;
use crate::rules::case::Case;
use crate::rules::effect::{Effect, EffectKind};
use crate::rules::expr::{self, Dice, Expr};
use crate::rules::roll::Outcome;
use crate::rules::roll::{MoveReport, Roll};
use crate::rules::rule::{EffectTemplate, LocationTo, NarrateTo, ResistSpec, Rule, Who};
use crate::Reason;

/// Why a rule's effects are expanded and landed, which decides what the landing journals.
#[derive(Clone, Copy)]
pub(crate) enum Cause {
    /// An intent's move this round: each effect takes an `EffectApplied` step, and an NPC's act
    /// is counted.
    Move,
    /// A timed action finishing: the actor does not move, so its `location` effects are left
    /// out; each other effect takes its step.
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
/// `max_effects_per_intent` is `BUDGET`, counting only those the `cause` keeps.
pub(crate) fn expand(
    case: &Case,
    rule: &Rule,
    cause: Cause,
    rolled: Option<Roll>,
    mut roller: Roller,
) -> Result<Expansion, Reason> {
    let rolled = match (rolled, &rule.roll) {
        (None, Some(spec)) => Some(roller.roll_spec(case, spec)?),
        (rolled, _) => rolled,
    };
    let outcome = rolled.as_ref().map(|out| out.outcome);
    let mut effects = expand_lists(case, rule, outcome, &mut roller)?;
    if matches!(cause, Cause::Finish) {
        effects.retain(|effect| !matches!(effect.kind, EffectKind::Location { .. }));
    }
    if effects.len() > case.canon.budget.max_effects_per_intent as usize {
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
pub(crate) fn preview(case: &Case, rule: &Rule) -> Result<Vec<Effect>, Reason> {
    expand_lists(case, rule, None, &mut PreviewDice)
}

/// `do`, then the list `outcome` picks: `success`, `partial` or `failure`. `None` means the
/// rule did not roll. `dice.resist` returns the subject's band; success shrugs the effect off.
fn expand_lists(
    case: &Case,
    rule: &Rule,
    outcome: Option<Outcome>,
    dice: &mut dyn Dice,
) -> Result<Vec<Effect>, Reason> {
    let mut selected = Vec::new();
    selected.extend(rule.do_.iter());
    match outcome {
        Some(Outcome::Success) => selected.extend(rule.success.iter()),
        Some(Outcome::Partial) => selected.extend(rule.partial.iter()),
        Some(Outcome::Miss) => selected.extend(rule.failure.iter()),
        None => {}
    }
    let mut effects = Vec::with_capacity(selected.len());
    let mut last_resist = None;
    for template in selected {
        expand_template(case, template, &mut effects, &mut last_resist, dice)?;
    }
    Ok(effects)
}

fn expand_template(
    case: &Case,
    template: &EffectTemplate,
    effects: &mut Vec<Effect>,
    last_resist: &mut Option<Outcome>,
    dice: &mut dyn Dice,
) -> Result<(), Reason> {
    let (canon, now, intent) = (case.canon, case.now, case.intent);
    let viewer = &intent.actor;
    if let Some(spec) = template.resist() {
        let shrugged = match spec {
            ResistSpec::Same => match last_resist {
                Some(Outcome::Success) => true,
                Some(_) => false,
                None => return Err(Reason::Schema),
            },
            ResistSpec::Roll {
                who,
                check,
                target,
                resisted,
            } => {
                let actor = expr::resolve_who_id(case, who)?;
                let number = expr::eval_with_dice(case, target, dice)?;
                let band = dice.resist(canon, now, &actor, check, number)?;
                *last_resist = Some(band);
                if band == Outcome::Success {
                    for alt in resisted {
                        expand_template(case, alt, effects, last_resist, dice)?;
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
        EffectTemplate::Flag {
            who, key, value, ..
        } => {
            let actor_id = if matches!(who, Who::World) {
                crate::rules::effect::WORLD.to_string()
            } else {
                expr::resolve_who_id(case, who)?
            };
            effects.push(Effect::new(
                viewer,
                &intent.id,
                EffectKind::Flag {
                    actor_id,
                    key: key.clone(),
                    value: *value,
                },
            ));
        }
        EffectTemplate::Location { to, teleport } => {
            let place = match to {
                LocationTo::Via(via) => {
                    let exit = crate::rules::travel::exit_via(case, via)?;
                    exit.ok_or(Reason::Schema)?.to.clone()
                }
                LocationTo::Place(id) if *teleport => id.clone(),
                LocationTo::Target if *teleport => match &intent.target {
                    Target::Place(id) => id.clone(),
                    _ => return Err(Reason::Schema),
                },
                LocationTo::Place(_) | LocationTo::Target => return Err(Reason::NotInSpec),
            };
            effects.push(location_effect(case, &place)?);
        }
        EffectTemplate::Meter {
            who, key, add, set, ..
        } => {
            let actor_id = expr::resolve_who_id(case, who)?;
            let write = eval_meter_write(case, &actor_id, key, add.as_ref(), set.as_ref(), dice)?;
            effects.push(Effect::new(
                viewer,
                &intent.id,
                EffectKind::Meter {
                    actor_id,
                    key: key.clone(),
                    write,
                },
            ));
        }
        EffectTemplate::Item {
            who, id, add, max, ..
        } => {
            let item_id = expr::resolve_item_id(case, id)?;
            let (actor_id, place) = resolve_holder(case, who)?;
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
        EffectTemplate::Wield { who, id } => {
            let actor_id = expr::resolve_who_id(case, who)?;
            effects.push(Effect::new(
                viewer,
                &intent.id,
                EffectKind::Wield {
                    actor_id,
                    id: expr::resolve_item_id(case, id)?,
                },
            ));
        }
        EffectTemplate::Unwield { who } => {
            let actor_id = expr::resolve_who_id(case, who)?;
            effects.push(Effect::new(
                viewer,
                &intent.id,
                EffectKind::Unwield { actor_id },
            ));
        }
        EffectTemplate::Clock { id, add, start } => {
            effects.push(Effect::new(
                viewer,
                &intent.id,
                EffectKind::Clock {
                    clock_id: id.clone(),
                    delta: add.unwrap_or(0),
                    start: *start,
                },
            ));
        }
        EffectTemplate::Note {
            id,
            to,
            place,
            fact_ids,
            text,
        } => {
            effects.push(Effect::new(
                viewer,
                &intent.id,
                EffectKind::Note {
                    note_id: id.clone(),
                    to: to.clone(),
                    place: place.clone(),
                    fact_ids: fact_ids.clone(),
                    text: text.clone(),
                },
            ));
        }
        EffectTemplate::Narrate {
            to, text, fact_ids, ..
        } => {
            let to = match to {
                NarrateTo::Here => now.actors_at(&expr::resolve_who_id(case, &Who::Here)?),
                NarrateTo::Self_ => vec![intent.actor.actor_id().to_string()],
                NarrateTo::Actors(ids) => ids.clone(),
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
        EffectTemplate::Memory {
            who,
            fact_ids,
            text,
        } => {
            let actor_id = expr::resolve_who_id(case, who)?;
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
        EffectTemplate::Reveal { who, fact } => {
            if matches!(who, Who::World) {
                return Err(Reason::Schema);
            }
            let actor_id = expr::resolve_who_id(case, who)?;
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

fn eval_meter_write(
    case: &Case,
    actor_id: &str,
    key: &str,
    add: Option<&Expr>,
    set: Option<&Expr>,
    dice: &mut dyn Dice,
) -> Result<MeterWrite, Reason> {
    if let Some(expr) = add {
        return Ok(MeterWrite::Offset(expr::eval_with_dice(case, expr, dice)?));
    }
    let desired = expr::eval_with_dice(case, set.ok_or(Reason::Schema)?, dice)?;
    let current = case.now.meter_value(case.canon, actor_id, key)?;
    Ok(MeterWrite::Offset(
        desired.checked_sub(current).ok_or(Reason::OutOfRange)?,
    ))
}

/// An item holder. `place:` and `here` are the place. Anyone else is an actor.
fn resolve_holder(case: &Case, who: &Who) -> Result<(Option<String>, Option<String>), Reason> {
    match who {
        Who::Place(id) => Ok((None, Some(id.clone()))),
        Who::Here => Ok((None, Some(expr::resolve_who_id(case, &Who::Here)?))),
        Who::World => Err(Reason::Schema),
        other => Ok((Some(expr::resolve_who_id(case, other)?), None)),
    }
}

fn location_effect(case: &Case, to: &str) -> Result<Effect, Reason> {
    let intent = case.intent;
    let from = case
        .now
        .actors
        .get(intent.actor.actor_id())
        .map(|actor| actor.place().to_string())
        .ok_or(Reason::UnknownId)?;
    Ok(Effect::new(
        &intent.actor,
        &intent.id,
        EffectKind::Location {
            actor_id: intent.actor.actor_id().to_string(),
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

    fn log(&mut self, _text: String) {}

    /// A resist roll that does not shrug the effect off.
    fn resist(
        &mut self,
        _: &Canon,
        _: &State,
        _: &str,
        _: &str,
        _: i64,
    ) -> Result<Outcome, Reason> {
        Ok(Outcome::Miss)
    }
}
