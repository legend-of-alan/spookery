use crate::canon::Canon;
use crate::check::Outcome;
use crate::effect::{Effect, EffectClass};
use crate::error::Reason;
use crate::intent::{Intent, Target, ViewerId};
use crate::meter::{self, MeterWrite};
use crate::model::Now;
use crate::rules::expr::{self, Ctx};
use crate::rules::gate;
use crate::rules::rule::{EffectTemplate, LocationTo, NarrateTo, Who};

pub fn expand(canon: &Canon, now: &Now, intent: &Intent) -> Result<Vec<Effect>, Reason> {
    expand_outcome(canon, now, intent, None)
}

/// `outcome` selects `success`, `partial`, or `failure`. `None` means the rule did not roll.
/// `do` is included either way.
pub fn expand_outcome(
    canon: &Canon,
    now: &Now,
    intent: &Intent,
    outcome: Option<Outcome>,
) -> Result<Vec<Effect>, Reason> {
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
    let mut effects = Vec::with_capacity(selected.len());
    for template in selected {
        expand_template(canon, now, &ctx, &viewer, template, &mut effects)?;
    }
    Ok(effects)
}

fn expand_template(
    canon: &Canon,
    now: &Now,
    ctx: &Ctx<'_>,
    viewer: &ViewerId,
    template: &EffectTemplate,
    effects: &mut Vec<Effect>,
) -> Result<(), Reason> {
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
        EffectTemplate::Stock { place, sku, add } => {
            effects.push(wrap(
                viewer,
                &intent.id,
                EffectClass::Stock {
                    place: place.clone(),
                    sku: sku.clone(),
                    delta: *add,
                },
            ));
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
                NarrateTo::Here => vec![expr::actor_of(now, ctx, &Who::Here)?],
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
        EffectTemplate::Item { .. } | EffectTemplate::Reveal { .. } => {
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
