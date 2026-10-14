use crate::canon::Canon;
use crate::check::Outcome;
use crate::effect::{Effect, EffectClass};
use crate::error::Reason;
use crate::intent::{Intent, Target, ViewerId};
use crate::meter::{self, MeterWrite};
use crate::model::Now;
use crate::rules::expr::{self, Ctx};
use crate::rules::gate;
use crate::rules::rule::{EffectTemplate, LocationTo, NarrateTo, ResistSpec, Who};

pub fn expand(canon: &Canon, now: &Now, intent: &Intent) -> Result<Vec<Effect>, Reason> {
    expand_outcome(canon, now, intent, None, ignore_resist)
}

/// A resist roll that does not shrug the effect off. Habit preview uses this.
pub fn ignore_resist(_actor: &str, _check: &str, _target: i64) -> Result<Outcome, Reason> {
    Ok(Outcome::Miss)
}

/// `outcome` selects `success`, `partial`, or `failure`. `None` means the rule did not roll.
/// `do` is included either way. `resist_roll` returns the subject's band. Success shrugs the effect off.
pub fn expand_outcome<F>(
    canon: &Canon,
    now: &Now,
    intent: &Intent,
    outcome: Option<Outcome>,
    mut resist_roll: F,
) -> Result<Vec<Effect>, Reason>
where
    F: FnMut(&str, &str, i64) -> Result<Outcome, Reason>,
{
    let rule = gate::resolve(canon, now, intent)?;
    let viewer = ViewerId::from_kind(intent.actor_kind, &intent.actor_id)?;
    let mut selected = Vec::new();
    selected.extend(rule.do_.iter());
    match outcome {
        Some(Outcome::Success) => selected.extend(rule.success.iter()),
        Some(Outcome::Partial) => selected.extend(rule.partial.iter()),
        Some(Outcome::Miss) => selected.extend(rule.failure.iter()),
        None => {}
    }
    let ctx = Ctx { intent };
    let expand = ExpandCtx {
        canon,
        now,
        ctx: &ctx,
        viewer: &viewer,
    };
    let mut effects = Vec::with_capacity(selected.len());
    let mut last_resist = None;
    for template in selected {
        expand_template(
            &expand,
            template,
            &mut effects,
            &mut last_resist,
            &mut resist_roll,
        )?;
    }
    Ok(effects)
}

struct ExpandCtx<'a> {
    canon: &'a Canon,
    now: &'a Now,
    ctx: &'a Ctx<'a>,
    viewer: &'a ViewerId,
}

fn expand_template<F>(
    expand: &ExpandCtx<'_>,
    template: &EffectTemplate,
    effects: &mut Vec<Effect>,
    last_resist: &mut Option<Outcome>,
    resist_roll: &mut F,
) -> Result<(), Reason>
where
    F: FnMut(&str, &str, i64) -> Result<Outcome, Reason>,
{
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
                let actor = expr::actor_of(now, ctx, who)?;
                let number = expr::eval(canon, now, ctx, target)?;
                let band = resist_roll(&actor, check, number)?;
                *last_resist = Some(band);
                if band == Outcome::Success {
                    for alt in resisted {
                        expand_template(expand, alt, effects, last_resist, resist_roll)?;
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
            let actor_id = expr::actor_of(now, ctx, who)?;
            effects.push(wrap(
                viewer,
                &intent.id,
                EffectClass::Flag {
                    actor_id,
                    key: key.clone(),
                    value: *value,
                },
            ));
        }
        EffectTemplate::Location { to, .. } => {
            let place = match to {
                LocationTo::Target => match &intent.target {
                    Target::Place(id) => id.clone(),
                    _ => return Err(Reason::Schema),
                },
                LocationTo::Place(id) => id.clone(),
                LocationTo::Via(_) => return Err(Reason::NotInSpec),
            };
            effects.push(location_effect(now, intent, &place)?);
        }
        EffectTemplate::Meter {
            who, key, add, set, ..
        } => {
            let actor_id = expr::actor_of(now, ctx, who)?;
            let write = meter_write(canon, now, &actor_id, key, *add, *set)?;
            effects.push(wrap(
                viewer,
                &intent.id,
                EffectClass::Meter {
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
            effects.push(wrap(
                viewer,
                &intent.id,
                EffectClass::Item {
                    actor_id,
                    place,
                    id: item_id,
                    delta: *add,
                    max: *max,
                },
            ));
        }
        EffectTemplate::Wield { who, id } => {
            let actor_id = expr::actor_of(now, ctx, who)?;
            effects.push(wrap(
                viewer,
                &intent.id,
                EffectClass::Wield {
                    actor_id,
                    id: expr::resolve_item_id(intent, id)?,
                },
            ));
        }
        EffectTemplate::Unwield { who } => {
            let actor_id = expr::actor_of(now, ctx, who)?;
            effects.push(wrap(viewer, &intent.id, EffectClass::Unwield { actor_id }));
        }
        EffectTemplate::Clock { id, add, .. } => {
            effects.push(wrap(
                viewer,
                &intent.id,
                EffectClass::Clock {
                    clock_id: id.clone(),
                    delta: add.unwrap_or(0),
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
            effects.push(wrap(
                viewer,
                &intent.id,
                EffectClass::Message {
                    message_id: id.clone(),
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
                NarrateTo::Self_ => vec![intent.actor_id.clone()],
                NarrateTo::Actors(ids) => ids.clone(),
            };
            effects.push(wrap(
                viewer,
                &intent.id,
                EffectClass::Narrate {
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
            let actor_id = expr::actor_of(now, ctx, who)?;
            effects.push(wrap(
                viewer,
                &intent.id,
                EffectClass::Memory {
                    actor_id,
                    fact_ids: fact_ids.clone(),
                    tags: Vec::new(),
                    text: text.clone(),
                },
            ));
        }
        EffectTemplate::Reveal { .. } => {
            return Err(Reason::NotInSpec);
        }
    }
    Ok(())
}

fn meter_write(
    canon: &Canon,
    now: &Now,
    actor_id: &str,
    key: &str,
    add: Option<i64>,
    set: Option<i64>,
) -> Result<MeterWrite, Reason> {
    if let Some(delta) = add {
        return Ok(MeterWrite::Offset(delta));
    }
    let desired = set.ok_or(Reason::Schema)?;
    let def = canon.meters.get(key).ok_or(Reason::UnknownKey)?;
    let live = now
        .actors
        .get(actor_id)
        .ok_or(Reason::UnknownKey)?
        .meters()
        .get(key)
        .ok_or(Reason::UnknownKey)?;
    let current = meter::current(def, live)?;
    Ok(MeterWrite::Offset(
        desired.checked_sub(current).ok_or(Reason::OutOfRange)?,
    ))
}

/// An item holder. `place:` and `here` are the place. Anyone else is an actor.
fn resolve_holder(
    now: &Now,
    ctx: &Ctx<'_>,
    who: &Who,
) -> Result<(Option<String>, Option<String>), Reason> {
    match who {
        Who::Place(id) => Ok((None, Some(id.clone()))),
        Who::Here => Ok((None, Some(expr::actor_of(now, ctx, &Who::Here)?))),
        Who::World => Err(Reason::Schema),
        other => Ok((Some(expr::actor_of(now, ctx, other)?), None)),
    }
}

fn resolve_actors_here(now: &Now, ctx: &Ctx<'_>) -> Result<Vec<String>, Reason> {
    let place = expr::actor_of(now, ctx, &Who::Here)?;
    let mut ids: Vec<String> = now
        .actors
        .iter()
        .filter(|(_, actor)| actor.place() == place)
        .map(|(id, _)| id.clone())
        .collect();
    ids.sort();
    Ok(ids)
}

fn wrap(viewer: &ViewerId, intent_id: &str, class: EffectClass) -> Effect {
    Effect {
        class,
        as_viewer: viewer.clone(),
        intent_id: intent_id.to_string(),
    }
}

fn location_effect(now: &Now, intent: &Intent, to: &str) -> Result<Effect, Reason> {
    let viewer = ViewerId::from_kind(intent.actor_kind, &intent.actor_id)?;
    let from = now
        .actors
        .get(&intent.actor_id)
        .map(|actor| actor.place().to_string())
        .ok_or(Reason::UnknownKey)?;
    Ok(Effect {
        class: EffectClass::Location {
            actor_id: intent.actor_id.clone(),
            from,
            to: to.to_string(),
        },
        as_viewer: viewer,
        intent_id: intent.id.clone(),
    })
}
