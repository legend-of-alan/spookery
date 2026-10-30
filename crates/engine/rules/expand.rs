//! Turn a rule template into effects for one intent.

use crate::canon::Canon;
use crate::effect::{Effect, EffectKind};
use crate::error::Reason;
use crate::intent::{Intent, Target, Viewer};
use crate::meter::MeterWrite;
use crate::model::{Presence, State};
use crate::rule::{EffectTemplate, LocationTo, NarrateTo, ResistSpec, Rule, TargetKind, When, Who};
use crate::rules::expr::{self, Ctx, Dice, Expr};
use crate::rules::gate;
use crate::rules::roll::Outcome;

/// `outcome` selects `success`, `partial`, or `failure`. `None` means the rule did not roll.
/// `do` is included either way. `dice.resist` returns the subject's band. Success shrugs the effect off.
pub fn expand_intent(
    canon: &Canon,
    now: &State,
    presence: &Presence,
    intent: &Intent,
    outcome: Option<Outcome>,
    dice: &mut dyn Dice,
) -> Result<Vec<Effect>, Reason> {
    let rule = gate::rule_for(canon, now, intent)?;
    rule.expand(canon, now, presence, intent, outcome, dice)
}

impl Rule {
    /// Expand this rule's lists. Change rules are not verbs, so they pass the rule in.
    pub fn expand(
        &self,
        canon: &Canon,
        now: &State,
        presence: &Presence,
        intent: &Intent,
        outcome: Option<Outcome>,
        dice: &mut dyn Dice,
    ) -> Result<Vec<Effect>, Reason> {
        let mut selected = Vec::new();
        selected.extend(self.do_.iter());
        match outcome {
            Some(Outcome::Success) => selected.extend(self.success.iter()),
            Some(Outcome::Partial) => selected.extend(self.partial.iter()),
            Some(Outcome::Miss) => selected.extend(self.failure.iter()),
            None => {}
        }
        let ctx = Ctx { intent, presence };
        let expand = ExpandCtx {
            canon,
            now,
            ctx: &ctx,
            viewer: &intent.actor,
        };
        let mut effects = Vec::with_capacity(selected.len());
        let mut last_resist = None;
        for template in selected {
            expand_template(&expand, template, &mut effects, &mut last_resist, dice)?;
        }
        Ok(effects)
    }
}

struct ExpandCtx<'a> {
    canon: &'a Canon,
    now: &'a State,
    ctx: &'a Ctx<'a>,
    viewer: &'a Viewer,
}

fn expand_template(
    expand: &ExpandCtx<'_>,
    template: &EffectTemplate,
    effects: &mut Vec<Effect>,
    last_resist: &mut Option<Outcome>,
    dice: &mut dyn Dice,
) -> Result<(), Reason> {
    let canon = expand.canon;
    let now = expand.now;
    let ctx = expand.ctx;
    let viewer = expand.viewer;
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
                let actor = expr::resolve_who_id(now, ctx, who)?;
                let number = expr::eval_with_dice(canon, now, ctx, target, dice)?.value;
                let band = dice.resist(canon, now, &actor, check, number)?;
                *last_resist = Some(band);
                if band == Outcome::Success {
                    for alt in resisted {
                        expand_template(expand, alt, effects, last_resist, dice)?;
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
    let intent = ctx.intent;
    match template {
        EffectTemplate::Flag {
            who, key, value, ..
        } => {
            let actor_id = if matches!(who, Who::World) {
                crate::effect::WORLD.to_string()
            } else {
                expr::resolve_who_id(now, ctx, who)?
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
                    let name = if via == "target" {
                        match &intent.target {
                            Target::Place(id) | Target::Exit(id) => id.clone(),
                            _ => return Err(Reason::Schema),
                        }
                    } else {
                        via.clone()
                    };
                    let from = now
                        .actors
                        .get(intent.actor.actor_id())
                        .map(|actor| actor.place())
                        .ok_or(Reason::UnknownId)?;
                    let exit =
                        crate::travel::find_exit(canon, from, &name).ok_or(Reason::NoExit)?;
                    exit.to.clone()
                }
                LocationTo::Place(id) if *teleport => id.clone(),
                LocationTo::Target if *teleport => match &intent.target {
                    Target::Place(id) | Target::Exit(id) => id.clone(),
                    _ => return Err(Reason::Schema),
                },
                LocationTo::Place(_) | LocationTo::Target => return Err(Reason::NotInSpec),
            };
            effects.push(location_effect(now, intent, &place)?);
        }
        EffectTemplate::Meter {
            who, key, add, set, ..
        } => {
            let actor_id = expr::resolve_who_id(now, ctx, who)?;
            let write = eval_meter_write(
                canon,
                now,
                &actor_id,
                key,
                MeterChange {
                    ctx,
                    add: add.as_ref(),
                    set: set.as_ref(),
                    dice,
                },
            )?;
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
            let item_id = expr::resolve_item_id(intent, id)?;
            let (actor_id, place) = resolve_holder(now, ctx, who)?;
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
            let actor_id = expr::resolve_who_id(now, ctx, who)?;
            effects.push(Effect::new(
                viewer,
                &intent.id,
                EffectKind::Wield {
                    actor_id,
                    id: expr::resolve_item_id(intent, id)?,
                },
            ));
        }
        EffectTemplate::Unwield { who } => {
            let actor_id = expr::resolve_who_id(now, ctx, who)?;
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
                NarrateTo::Here => resolve_actors_here(now, ctx)?,
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
            let actor_id = expr::resolve_who_id(now, ctx, who)?;
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
            let actor_id = expr::resolve_who_id(now, ctx, who)?;
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

struct MeterChange<'a> {
    ctx: &'a Ctx<'a>,
    add: Option<&'a Expr>,
    set: Option<&'a Expr>,
    dice: &'a mut dyn Dice,
}

fn eval_meter_write(
    canon: &Canon,
    now: &State,
    actor_id: &str,
    key: &str,
    change: MeterChange<'_>,
) -> Result<MeterWrite, Reason> {
    if let Some(expr) = change.add {
        let delta = expr::eval_with_dice(canon, now, change.ctx, expr, change.dice)?.value;
        return Ok(MeterWrite::Offset(delta));
    }
    let desired = expr::eval_with_dice(
        canon,
        now,
        change.ctx,
        change.set.ok_or(Reason::Schema)?,
        change.dice,
    )?
    .value;
    let current = now.meter_value(canon, actor_id, key)?;
    Ok(MeterWrite::Offset(
        desired.checked_sub(current).ok_or(Reason::OutOfRange)?,
    ))
}

/// An item holder. `place:` and `here` are the place. Anyone else is an actor.
fn resolve_holder(
    now: &State,
    ctx: &Ctx<'_>,
    who: &Who,
) -> Result<(Option<String>, Option<String>), Reason> {
    match who {
        Who::Place(id) => Ok((None, Some(id.clone()))),
        Who::Here => Ok((None, Some(expr::resolve_who_id(now, ctx, &Who::Here)?))),
        Who::World => Err(Reason::Schema),
        other => Ok((Some(expr::resolve_who_id(now, ctx, other)?), None)),
    }
}

fn resolve_actors_here(now: &State, ctx: &Ctx<'_>) -> Result<Vec<String>, Reason> {
    let place = expr::resolve_who_id(now, ctx, &Who::Here)?;
    let mut ids: Vec<String> = now
        .actors
        .iter()
        .filter(|(_, actor)| actor.place() == place)
        .map(|(id, _)| id.clone())
        .collect();
    ids.sort();
    Ok(ids)
}

fn location_effect(now: &State, intent: &Intent, to: &str) -> Result<Effect, Reason> {
    let from = now
        .actors
        .get(intent.actor.actor_id())
        .map(|actor| actor.place().to_string())
        .ok_or(Reason::UnknownId)?;
    Ok(Effect {
        kind: EffectKind::Location {
            actor_id: intent.actor.actor_id().to_string(),
            from,
            to: to.to_string(),
            arrived_at: None,
        },
        viewer: intent.actor.clone(),
        intent_id: intent.id.clone(),
    })
}

pub fn stamp_for_priority(canon: &Canon, now: &State, presence: &Presence, intent: &mut Intent) {
    let Ok(effects) = expand_intent(canon, now, presence, intent, None, &mut PreviewDice) else {
        return;
    };
    // A verb with no target must stay empty. Filling it from a location effect
    // makes the later gate call report SCHEMA.
    let keeps_empty = gate::rule_for(canon, now, intent).ok().is_some_and(|rule| {
        matches!(
            &rule.when,
            When::Verb {
                target: TargetKind::None,
                ..
            }
        )
    });
    if keeps_empty {
        for effect in &effects {
            if let EffectKind::Note { to, .. } = &effect.kind {
                intent.args.message_to = Some(to.clone());
            }
        }
        return;
    }
    stamp(intent, &effects);
}

/// Faces of 1, and no journal line. Gather only needs the effect shape.
/// The round rolls the real faces when it applies the intent.
pub(crate) struct PreviewDice;

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

pub(crate) fn stamp(intent: &mut Intent, effects: &[Effect]) {
    for effect in effects {
        if intent.target == Target::Empty {
            match &effect.kind {
                EffectKind::Location { to, .. } => {
                    intent.target = Target::Place(to.clone());
                }
                EffectKind::Clock { clock_id, .. } => {
                    intent.target = Target::Clock(clock_id.clone());
                }
                _ => {}
            }
        }
        if let EffectKind::Note { to, .. } = &effect.kind {
            intent.args.message_to = Some(to.clone());
        }
    }
}
