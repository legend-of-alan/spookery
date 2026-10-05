use crate::canon::{Canon, NoteDef, TickProfile};
use crate::effect::EffectClass;
use crate::firewall::allows;
use crate::intent::{ActorKind, ViewerId};
use crate::limits::MINUTES_PER_DAY;
use crate::model::{Now, Rumor};

pub(crate) fn stock_walk(
    canon: &Canon,
    now: &Now,
    actor_id: &str,
) -> Option<(String, String, i64)> {
    let place = now.actors.get(actor_id)?.place().to_string();
    let rates = canon.profile(actor_id)?.stock_rate.get(&place)?;
    for (sku, rate) in rates {
        let stock = now
            .places
            .get(&place)
            .and_then(|place_live| place_live.stock.get(sku))
            .copied()
            .unwrap_or(0);
        if stock.checked_add(*rate).is_some_and(|next| next >= 0) {
            return Some((place, sku.clone(), *rate));
        }
    }
    None
}

pub(crate) fn first_owned_clock(now: &Now, actor_id: &str) -> Option<String> {
    let humans = now.presence.humans;
    now.clocks
        .iter()
        .find(|(_, clock)| {
            clock.owners.iter().any(|owner| owner == actor_id)
                && !clock.paused
                && clock.value < clock.max
                && !(clock.requires_presence && humans == 0)
        })
        .map(|(id, _)| id.clone())
}

pub(crate) fn first_due_note<'a>(now: &Now, profile: &'a TickProfile) -> Option<&'a NoteDef> {
    profile.notes.iter().find(|note| {
        now.clocks
            .get(&note.due_clock)
            .is_some_and(|clock| clock.value >= note.due_value)
            && !now.messages.iter().any(|message| message.id == note.id)
    })
}

pub(crate) fn first_drift(now: &Now, actor_id: &str) -> Option<(String, i64)> {
    let actor = now.actors.get(actor_id)?;
    if actor.busy() {
        return None;
    }
    let relations = actor.relationships()?;
    let mut pairs: Vec<(String, i64)> = relations
        .iter()
        .filter(|(other, relation)| {
            relation.stance != relation.target
                && now
                    .world_time
                    .saturating_sub(relation.last_interaction_world_time)
                    >= MINUTES_PER_DAY
                && now
                    .actors
                    .get(*other)
                    .is_some_and(|other_actor| !other_actor.busy())
        })
        .map(|(other, relation)| {
            let delta = (relation.target - relation.stance).clamp(-1, 1);
            (other.clone(), delta)
        })
        .collect();
    pairs.sort_by(|left, right| left.0.cmp(&right.0));
    pairs.into_iter().next()
}

pub(crate) fn rumor_class(canon: &Canon, now: &Now, actor_id: &str) -> Option<EffectClass> {
    if canon.budget.max_rumor_hops == 0 {
        return None;
    }
    let place = now.actors.get(actor_id)?.place().to_string();
    let mut rumors: Vec<&Rumor> = now
        .rumors
        .iter()
        .filter(|rumor| rumor.heat >= 1 && rumor.known_by.iter().any(|id| id == actor_id))
        .collect();
    rumors.sort_by(|left, right| left.id.cmp(&right.id));
    for rumor in rumors {
        let mut recipients: Vec<String> = now
            .actors
            .keys()
            .filter(|other| *other != actor_id)
            .filter(|other| adjacent(now, actor_id, other, &place))
            .filter(|other| {
                let kind = match now.actors.get(*other) {
                    Some(actor) if actor.is_npc() => ActorKind::Npc,
                    Some(actor) if actor.is_pc() => ActorKind::Player,
                    _ => return false,
                };
                let Ok(viewer) = ViewerId::from_kind(kind, other) else {
                    return false;
                };
                rumor
                    .fact_ids
                    .iter()
                    .all(|fact| allows(canon, now, &viewer, fact))
            })
            .cloned()
            .collect();
        recipients.sort();
        if let Some(to) = recipients.into_iter().next() {
            return Some(EffectClass::Rumor {
                rumor_id: rumor.id.clone(),
                fact_ids: rumor.fact_ids.clone(),
                from: actor_id.to_string(),
                to,
                heat: rumor.heat,
            });
        }
    }
    None
}

fn adjacent(now: &Now, actor_id: &str, other: &str, place: &str) -> bool {
    let same_place = now
        .actors
        .get(other)
        .is_some_and(|actor| actor.place() == place);
    let related = now
        .actors
        .get(actor_id)
        .and_then(|actor| actor.relationships())
        .is_some_and(|relations| relations.contains_key(other));
    same_place || related
}
