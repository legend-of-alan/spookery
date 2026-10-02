use crate::canon::{blake2s_hex, Canon};
use crate::eligibility::is_eligible;
use crate::error::Reason;
use crate::firewall::allows;
use crate::limits::{world_hour, MINUTES_PER_DAY};
use crate::model::{
    ActorKind, ActorLive, Intent, IntentArgs, Now, RunState, Source, Target, ViewerId,
};

pub fn kernel_intent_id(
    shard_id: &str,
    tick_index: u64,
    actor_id: &str,
    move_name: &str,
) -> String {
    blake2s_hex(format!("{shard_id}|{tick_index}|{actor_id}|{move_name}").as_bytes())
}

pub fn kernel_intents_for_eligible_npcs(canon: &Canon, now: &Now) -> Vec<Intent> {
    let mut intents = Vec::new();
    for actor_id in now.actors.keys() {
        if !is_eligible(canon, now, actor_id) {
            continue;
        }
        if let Some(intent) = derive_one(canon, now, actor_id) {
            intents.push(intent);
        }
    }
    intents
}

pub fn season_intent(now: &Now) -> Option<Intent> {
    let (actor_id, actor_kind) = season_actor(now)?;
    Some(mint(
        now,
        actor_id,
        actor_kind,
        Source::System,
        "file_proposal",
        Target::Empty,
        IntentArgs {
            proposal_id: Some("season_advance".to_string()),
            kind: Some("other".to_string()),
            target: Some(String::new()),
            summary: Some("season_advance".to_string()),
            ..IntentArgs::default()
        },
    ))
}

pub fn filter_blocklist(intents: Vec<Intent>, humans: u32) -> (Vec<Intent>, Vec<(Intent, Reason)>) {
    if humans > 0 {
        return (intents, Vec::new());
    }
    let mut kept = Vec::new();
    let mut rejected = Vec::new();
    for intent in intents {
        if matches!(
            intent.move_name.as_str(),
            "talk" | "attempt" | "attack" | "give" | "take"
        ) {
            rejected.push((intent, Reason::LiveOnly));
        } else {
            kept.push(intent);
        }
    }
    (kept, rejected)
}

pub fn allowed(canon: &Canon, now: &Now, intent: &Intent) -> Result<(), Reason> {
    if !canon.moves.contains_key(&intent.move_name) {
        return Err(Reason::UnknownMove);
    }
    match &intent.target {
        Target::Empty => {}
        Target::Place(id) => {
            if !canon.places.contains_key(id) && !now.places.contains_key(id) {
                return Err(Reason::UnknownKey);
            }
        }
        Target::Actor(id) => {
            if !now.actors.contains_key(id) {
                return Err(Reason::UnknownKey);
            }
        }
        Target::Clock(id) => {
            if !now.clocks.contains_key(id) {
                return Err(Reason::UnknownKey);
            }
        }
        Target::Rumor(id) => {
            if !now.rumors.iter().any(|rumor| rumor.id == *id) {
                return Err(Reason::UnknownKey);
            }
        }
    }
    if let Some(facts) = &intent.args.fact_ids {
        let viewer = ViewerId::from_kind(intent.actor_kind, &intent.actor_id)?;
        for fact in facts {
            if !allows(canon, now, &viewer, fact) {
                return Err(Reason::IllegalKnowledge);
            }
        }
    }
    if intent.source == Source::Kernel && intent.actor_kind == ActorKind::Player {
        return Err(Reason::NotOwner);
    }
    if intent.source == Source::Kernel
        && intent.actor_kind == ActorKind::Npc
        && !is_eligible(canon, now, &intent.actor_id)
    {
        return Err(Reason::NotOwner);
    }
    Ok(())
}

fn season_actor(now: &Now) -> Option<(String, ActorKind)> {
    if let Some(id) = now
        .actors
        .iter()
        .find(|(_, actor)| actor.is_pc())
        .map(|(id, _)| id.clone())
    {
        return Some((id, ActorKind::Player));
    }
    let mut owners: Vec<String> = now
        .clocks
        .values()
        .flat_map(|clock| clock.owners.iter().cloned())
        .filter(|id| now.actors.get(id).is_some_and(|actor| actor.is_npc()))
        .collect();
    owners.sort();
    owners.dedup();
    owners.into_iter().next().map(|id| (id, ActorKind::Npc))
}

fn derive_one(canon: &Canon, now: &Now, actor_id: &str) -> Option<Intent> {
    let profile = canon.profile(actor_id)?;
    let actor = now.actors.get(actor_id)?;
    let place = actor.place().to_string();
    let hour = world_hour(now.world_time);

    if let Some(intent) = restock(canon, now, actor_id, &place) {
        return Some(intent);
    }
    if now.pause.travel == RunState::Running {
        if let Some(row) = profile.winning_row(hour) {
            if row.place != place && profile.range.iter().any(|item| item == &row.place) {
                return Some(mint(
                    now,
                    actor_id.to_string(),
                    ActorKind::Npc,
                    Source::Kernel,
                    "travel",
                    Target::Place(row.place.clone()),
                    IntentArgs::default(),
                ));
            }
        }
    }
    if let Some(row) = profile.winning_row(hour) {
        if row.place == place {
            match row.activity.as_deref() {
                Some("work") => {
                    return Some(mint(
                        now,
                        actor_id.to_string(),
                        ActorKind::Npc,
                        Source::Kernel,
                        "work",
                        Target::Empty,
                        IntentArgs::default(),
                    ));
                }
                Some("rest") => {
                    return Some(mint(
                        now,
                        actor_id.to_string(),
                        ActorKind::Npc,
                        Source::Kernel,
                        "rest",
                        Target::Empty,
                        IntentArgs {
                            delta: row.rest_delta,
                            ..IntentArgs::default()
                        },
                    ));
                }
                _ => {}
            }
        }
    }
    if now.pause.relationship_drift == RunState::Running {
        if let Some(intent) = drift(now, actor_id) {
            return Some(intent);
        }
    }
    if let Some(intent) = spread(canon, now, actor_id, &place) {
        return Some(intent);
    }
    if let Some(intent) = leave_note(now, actor_id, profile) {
        return Some(intent);
    }
    advance_clock(now, actor_id)
}

fn restock(canon: &Canon, now: &Now, actor_id: &str, place: &str) -> Option<Intent> {
    let profile = canon.profile(actor_id)?;
    let rates = profile.stock_rate.get(place)?;
    for (sku, rate) in rates {
        let stock = now
            .places
            .get(place)
            .and_then(|place_live| place_live.stock.get(sku))
            .copied()
            .unwrap_or(0);
        if stock.checked_add(*rate).is_some_and(|next| next >= 0) {
            return Some(mint(
                now,
                actor_id.to_string(),
                ActorKind::Npc,
                Source::Kernel,
                "restock",
                Target::Empty,
                IntentArgs {
                    sku: Some(sku.clone()),
                    delta: Some(*rate),
                    ..IntentArgs::default()
                },
            ));
        }
    }
    None
}

fn drift(now: &Now, actor_id: &str) -> Option<Intent> {
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
    let (other, delta) = pairs.into_iter().next()?;
    Some(mint(
        now,
        actor_id.to_string(),
        ActorKind::Npc,
        Source::Kernel,
        "drift_relationship",
        Target::Actor(other),
        IntentArgs {
            delta: Some(delta),
            ..IntentArgs::default()
        },
    ))
}

fn spread(canon: &Canon, now: &Now, actor_id: &str, place: &str) -> Option<Intent> {
    if canon.budget.max_rumor_hops == 0 {
        return None;
    }
    let mut rumors: Vec<&crate::model::Rumor> = now
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
            .filter(|other| adjacent(now, actor_id, other, place))
            .filter(|other| {
                let kind = match now.actors.get(*other) {
                    Some(ActorLive::Npc { .. }) => ActorKind::Npc,
                    Some(ActorLive::Pc { .. }) => ActorKind::Player,
                    None => return false,
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
            return Some(mint(
                now,
                actor_id.to_string(),
                ActorKind::Npc,
                Source::Kernel,
                "spread_rumor",
                Target::Rumor(rumor.id.clone()),
                IntentArgs {
                    to: Some(to),
                    ..IntentArgs::default()
                },
            ));
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

fn leave_note(now: &Now, actor_id: &str, profile: &crate::canon::TickProfile) -> Option<Intent> {
    for note in &profile.notes {
        let due = now
            .clocks
            .get(&note.due_clock)
            .is_some_and(|clock| clock.value >= note.due_value);
        let fresh = !now.messages.iter().any(|message| message.id == note.id);
        if due && fresh {
            return Some(mint(
                now,
                actor_id.to_string(),
                ActorKind::Npc,
                Source::Kernel,
                "leave_note",
                Target::Empty,
                IntentArgs {
                    message_id: Some(note.id.clone()),
                    due_clock: Some(note.due_clock.clone()),
                    due_value: Some(note.due_value),
                    message_to: Some(note.to.clone()),
                    place: Some(note.place.clone()),
                    fact_ids: Some(note.fact_ids.clone()),
                    text: Some(note.text.clone()),
                    ..IntentArgs::default()
                },
            ));
        }
    }
    None
}

fn advance_clock(now: &Now, actor_id: &str) -> Option<Intent> {
    let mut clocks: Vec<&String> = now
        .clocks
        .iter()
        .filter(|(_, clock)| {
            clock.owners.iter().any(|owner| owner == actor_id)
                && !clock.paused
                && clock.value < clock.max
        })
        .map(|(id, _)| id)
        .collect();
    clocks.sort();
    let clock_id = clocks.into_iter().next()?.clone();
    Some(mint(
        now,
        actor_id.to_string(),
        ActorKind::Npc,
        Source::Kernel,
        "advance_clock",
        Target::Clock(clock_id),
        IntentArgs {
            delta: Some(1),
            ..IntentArgs::default()
        },
    ))
}

fn mint(
    now: &Now,
    actor_id: String,
    actor_kind: ActorKind,
    source: Source,
    move_name: &str,
    target: Target,
    args: IntentArgs,
) -> Intent {
    Intent {
        id: kernel_intent_id(&now.shard_id, now.tick_index, &actor_id, move_name),
        tick_index: now.tick_index,
        actor_id,
        actor_kind,
        source,
        move_name: move_name.to_string(),
        target,
        args,
        rationale_code: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_intent_id_matches_pin_p37() {
        assert_eq!(
            kernel_intent_id("cap", 1, "n2", "work"),
            "daa186f67613ae8ea1275b3fd03c38d52eea99e976235e499b5c2b0853c0d819"
        );
    }
}
