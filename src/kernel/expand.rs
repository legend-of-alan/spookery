use crate::canon::Canon;
use crate::error::Reason;
use crate::model::{Effect, Intent, Now, Target, ViewerId};
use crate::moves::{ActionRow, MoveOn, RowEffect};

pub fn expand(canon: &Canon, now: &Now, intent: &Intent) -> Result<Vec<Effect>, Reason> {
    expand_outcome(canon, now, intent, None)
}

/// `outcome` selects `success` (`Some(true)`) or `failure` (`Some(false)`).
/// `None` means the row did not roll. `effects` is included either way.
pub fn expand_outcome(
    canon: &Canon,
    now: &Now,
    intent: &Intent,
    outcome: Option<bool>,
) -> Result<Vec<Effect>, Reason> {
    if let Some(row) = canon
        .moves
        .get(&intent.move_name)
        .and_then(|def| def.row.as_ref())
    {
        return expand_row(now, intent, row, outcome);
    }
    let viewer = ViewerId::from_kind(intent.actor_kind, &intent.actor_id)?;
    let intent_id = intent.id.clone();
    match intent.move_name.as_str() {
        "wait" | "attempt" => Ok(Vec::new()),
        "talk" | "attack" | "give" | "take" => Err(Reason::NotInSpec),
        "travel" => {
            let Target::Place(to) = &intent.target else {
                return Err(Reason::Schema);
            };
            Ok(vec![location_effect(now, intent, to)?])
        }
        "work" => {
            let Some(profile) = canon.profile(&intent.actor_id) else {
                return Ok(Vec::new());
            };
            let Some(work) = &profile.work else {
                return Ok(Vec::new());
            };
            let mut effects = Vec::new();
            for item in &work.needs {
                effects.push(Effect::Need {
                    as_viewer: viewer.clone(),
                    intent_id: intent_id.clone(),
                    actor_id: intent.actor_id.clone(),
                    key: item.key.clone(),
                    delta: item.delta,
                });
            }
            for item in &work.resources {
                effects.push(Effect::Resource {
                    as_viewer: viewer.clone(),
                    intent_id: intent_id.clone(),
                    actor_id: intent.actor_id.clone(),
                    key: item.key.clone(),
                    delta: item.delta,
                });
            }
            Ok(effects)
        }
        "restock" => {
            let sku = intent.args.sku.clone().ok_or(Reason::UnknownKey)?;
            let delta = intent.args.delta.ok_or(Reason::UnknownKey)?;
            let place = now
                .actors
                .get(&intent.actor_id)
                .map(|actor| actor.place().to_string())
                .ok_or(Reason::UnknownKey)?;
            Ok(vec![Effect::Stock {
                as_viewer: viewer,
                intent_id,
                place,
                sku,
                delta,
            }])
        }
        "rest" => {
            let delta = intent.args.delta.ok_or(Reason::UnknownKey)?;
            Ok(vec![Effect::Need {
                as_viewer: viewer,
                intent_id,
                actor_id: intent.actor_id.clone(),
                key: "rest".to_string(),
                delta,
            }])
        }
        "drift_relationship" => {
            let Target::Actor(other) = &intent.target else {
                return Err(Reason::Schema);
            };
            let delta = intent.args.delta.ok_or(Reason::UnknownKey)?;
            Ok(vec![Effect::Relationship {
                as_viewer: viewer,
                intent_id,
                a: intent.actor_id.clone(),
                b: other.clone(),
                delta,
            }])
        }
        "spread_rumor" => {
            let Target::Rumor(rumor_id) = &intent.target else {
                return Err(Reason::Schema);
            };
            let rumor_id = rumor_id.clone();
            let to = intent.args.to.clone().ok_or(Reason::UnknownKey)?;
            let rumor = now
                .rumors
                .iter()
                .find(|rumor| rumor.id == rumor_id)
                .ok_or(Reason::UnknownKey)?;
            Ok(vec![Effect::Rumor {
                as_viewer: viewer,
                intent_id,
                rumor_id,
                fact_ids: rumor.fact_ids.clone(),
                from: intent.actor_id.clone(),
                to,
                heat: rumor.heat,
            }])
        }
        "leave_note" => Ok(vec![Effect::Message {
            as_viewer: viewer,
            intent_id,
            message_id: intent.args.message_id.clone().ok_or(Reason::UnknownKey)?,
            to: intent.args.message_to.clone().unwrap_or_default(),
            place: intent.args.place.clone(),
            fact_ids: intent.args.fact_ids.clone().unwrap_or_default(),
            text: intent.args.text.clone().ok_or(Reason::UnknownKey)?,
        }]),
        "file_proposal" => Ok(vec![Effect::Proposal {
            as_viewer: viewer,
            intent_id,
            proposal_id: intent.args.proposal_id.clone().ok_or(Reason::UnknownKey)?,
            kind: intent.args.kind.clone().ok_or(Reason::UnknownKey)?,
            target: intent.args.target.clone().unwrap_or_default(),
            summary: intent.args.summary.clone().ok_or(Reason::UnknownKey)?,
        }]),
        "advance_clock" => {
            let Target::Clock(clock_id) = &intent.target else {
                return Err(Reason::Schema);
            };
            Ok(vec![Effect::Clock {
                as_viewer: viewer,
                intent_id,
                clock_id: clock_id.clone(),
                delta: intent.args.delta.unwrap_or(1),
            }])
        }
        other => {
            if canon.moves.contains_key(other) {
                Err(Reason::NotInSpec)
            } else {
                Err(Reason::UnknownMove)
            }
        }
    }
}

/// One effect per selected template. `effects` always. `success` or `failure` when that outcome was rolled.
/// A flag's `speaker` is the acting actor and `named` is the typed id.
/// A location template is the same effect as the travel arm: `to` is the typed place.
fn expand_row(
    now: &Now,
    intent: &Intent,
    row: &ActionRow,
    outcome: Option<bool>,
) -> Result<Vec<Effect>, Reason> {
    let viewer = ViewerId::from_kind(intent.actor_kind, &intent.actor_id)?;
    let mut selected = Vec::with_capacity(row.effects.len() + row.success.len().max(row.failure.len()));
    selected.extend(row.effects.iter());
    match outcome {
        Some(true) => selected.extend(row.success.iter()),
        Some(false) => selected.extend(row.failure.iter()),
        None => {}
    }
    let mut effects = Vec::with_capacity(selected.len());
    for template in selected {
        match template {
            RowEffect::Flag { key, value } => {
                let actor_id = match row.on {
                    MoveOn::Speaker => intent.actor_id.clone(),
                    MoveOn::Named => match &intent.target {
                        Target::Actor(id) => id.clone(),
                        _ => return Err(Reason::Schema),
                    },
                };
                effects.push(Effect::Flag {
                    as_viewer: viewer.clone(),
                    intent_id: intent.id.clone(),
                    actor_id,
                    key: key.clone(),
                    value: *value,
                });
            }
            RowEffect::Location => {
                if row.on != MoveOn::Speaker {
                    return Err(Reason::Schema);
                }
                let Target::Place(to) = &intent.target else {
                    return Err(Reason::Schema);
                };
                effects.push(location_effect(now, intent, to)?);
            }
            RowEffect::Resource { key, delta } => {
                let actor_id = match row.on {
                    MoveOn::Speaker => intent.actor_id.clone(),
                    MoveOn::Named => match &intent.target {
                        Target::Actor(id) => id.clone(),
                        _ => return Err(Reason::Schema),
                    },
                };
                effects.push(Effect::Resource {
                    as_viewer: viewer.clone(),
                    intent_id: intent.id.clone(),
                    actor_id,
                    key: key.clone(),
                    delta: *delta,
                });
            }
        }
    }
    Ok(effects)
}

fn location_effect(now: &Now, intent: &Intent, to: &str) -> Result<Effect, Reason> {
    let viewer = ViewerId::from_kind(intent.actor_kind, &intent.actor_id)?;
    let from = now
        .actors
        .get(&intent.actor_id)
        .map(|actor| actor.place().to_string())
        .ok_or(Reason::UnknownKey)?;
    Ok(Effect::Location {
        as_viewer: viewer,
        intent_id: intent.id.clone(),
        actor_id: intent.actor_id.clone(),
        from,
        to: to.to_string(),
    })
}
