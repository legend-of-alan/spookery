use crate::effect::{Effect, EffectClass};
use crate::error::Reason;
use crate::firewall::allows;
use crate::inventory::{clamp_need, resource_next, stock_next};
use crate::key::StateKey;
use crate::limits::MINUTES_PER_DAY;
use crate::model::{ActorKind, Intent, MemoryLine, Message, ProposalDoc, Reputation, ViewerId};
use crate::shard::Txn;

pub fn apply_effect(txn: &mut Txn, intent: &Intent, effect: Effect) -> Result<(), Reason> {
    if !viewer_matches(intent, &effect) {
        return Err(Reason::ViewerMismatch);
    }
    match effect.class {
        EffectClass::Clock { clock_id, delta } => apply_clock(txn, &clock_id, delta),
        EffectClass::Need {
            actor_id,
            key,
            delta,
        } => apply_need(txn, &actor_id, &key, delta),
        EffectClass::Stock { place, sku, delta } => apply_stock(txn, intent, &place, &sku, delta),
        EffectClass::Location { actor_id, from, to } => {
            apply_location(txn, intent, &actor_id, &from, &to)
        }
        EffectClass::Relationship { a, b, delta } => apply_relationship(txn, &a, &b, delta),
        EffectClass::Reputation {
            actor_id,
            faction,
            delta,
        } => apply_reputation(txn, &actor_id, &faction, delta),
        EffectClass::Memory {
            actor_id,
            fact_ids,
            tags,
            text,
        } => apply_memory(txn, intent, &actor_id, fact_ids, tags, text),
        EffectClass::Rumor {
            rumor_id,
            fact_ids,
            to,
            heat,
            ..
        } => apply_rumor(txn, &rumor_id, &fact_ids, &to, heat),
        EffectClass::Resource {
            actor_id,
            key,
            delta,
        } => apply_resource(txn, &actor_id, &key, delta),
        EffectClass::Flag {
            actor_id,
            key,
            value,
        } => apply_flag(txn, &actor_id, &key, value),
        EffectClass::Proposal {
            proposal_id,
            kind,
            target,
            summary,
        } => apply_proposal(txn, intent, &proposal_id, &kind, &target, &summary),
        EffectClass::Message {
            message_id,
            to,
            place,
            fact_ids,
            text,
        } => apply_message(txn, message_id, to, place, fact_ids, text),
    }
}

fn viewer_matches(intent: &Intent, effect: &Effect) -> bool {
    match (intent.actor_kind, effect.as_viewer()) {
        (ActorKind::Npc, ViewerId::Npc(id)) => id == &intent.actor_id,
        (ActorKind::Player, ViewerId::Player(id)) => id == &intent.actor_id,
        _ => false,
    }
}

fn apply_clock(txn: &mut Txn, clock_id: &str, delta: i64) -> Result<(), Reason> {
    let humans = txn.now().presence.humans;
    let clock = txn.now().clocks.get(clock_id).ok_or(Reason::UnknownKey)?;
    if clock.paused || (clock.requires_presence && humans == 0) {
        return Err(Reason::PausedClock);
    }
    let current = clock.value;
    let next = current.checked_add(delta).ok_or(Reason::OutOfRange)?;
    if next < 0 || next > clock.max {
        return Err(Reason::OutOfRange);
    }
    if next != current {
        txn.track(StateKey::ClockValue(clock_id.to_string()))?;
    }
    txn.now_mut()
        .clocks
        .get_mut(clock_id)
        .ok_or(Reason::UnknownKey)?
        .value = next;
    Ok(())
}

fn apply_need(txn: &mut Txn, actor_id: &str, key: &str, delta: i64) -> Result<(), Reason> {
    let bound = txn
        .canon()
        .profile(actor_id)
        .and_then(|profile| profile.needs.get(key))
        .ok_or(Reason::UnknownKey)?;
    let min = bound.min;
    let max = bound.max;
    let current = txn
        .now()
        .actors
        .get(actor_id)
        .and_then(|actor| actor.needs())
        .and_then(|needs| needs.get(key))
        .copied()
        .unwrap_or(0);
    let next = clamp_need(current, delta, min, max)?;
    if next != current {
        txn.track(StateKey::ActorNeed(actor_id.to_string(), key.to_string()))?;
    }
    txn.now_mut()
        .actors
        .get_mut(actor_id)
        .and_then(|actor| actor.needs_mut())
        .ok_or(Reason::UnknownKey)?
        .insert(key.to_string(), next);
    Ok(())
}

fn apply_stock(
    txn: &mut Txn,
    intent: &Intent,
    place: &str,
    sku: &str,
    delta: i64,
) -> Result<(), Reason> {
    let profile = txn
        .canon()
        .profile(&intent.actor_id)
        .ok_or(Reason::NoProfile)?;
    if !profile.range.iter().any(|item| item == place) || !profile.on_graph(place) {
        return Err(Reason::OutOfRange);
    }
    let rate = profile
        .stock_rate
        .get(place)
        .and_then(|rates| rates.get(sku))
        .copied()
        .ok_or(Reason::OutOfRange)?;
    if delta.abs() > rate.abs() {
        return Err(Reason::OutOfRange);
    }
    let current = txn
        .now()
        .places
        .get(place)
        .and_then(|place_live| place_live.stock.get(sku))
        .copied()
        .unwrap_or(0);
    let next = stock_next(current, delta)?;
    if next != current {
        txn.track(StateKey::PlaceStock(place.to_string(), sku.to_string()))?;
    }
    let place_live = txn
        .now_mut()
        .places
        .get_mut(place)
        .ok_or(Reason::OutOfRange)?;
    place_live.stock.insert(sku.to_string(), next);
    Ok(())
}

fn apply_location(
    txn: &mut Txn,
    intent: &Intent,
    actor_id: &str,
    from: &str,
    to: &str,
) -> Result<(), Reason> {
    if actor_id != intent.actor_id {
        return Err(Reason::ActorMismatch);
    }
    let actor = txn.now().actors.get(actor_id).ok_or(Reason::UnknownKey)?;
    if txn.now().presence.humans == 0 && actor.is_pc() {
        return Err(Reason::TableEmpty);
    }
    if actor.place() != from {
        return Err(Reason::OutOfRange);
    }
    let profile = txn.canon().profile(actor_id).ok_or(Reason::NoProfile)?;
    if !profile.range.iter().any(|item| item == to) || !profile.on_graph(to) {
        return Err(Reason::OutOfRange);
    }
    let place_changed = actor.place() != to;
    if place_changed {
        txn.track(StateKey::ActorPlace(actor_id.to_string()))?;
    }
    *txn.now_mut()
        .actors
        .get_mut(actor_id)
        .ok_or(Reason::UnknownKey)?
        .place_mut() = to.to_string();
    Ok(())
}

fn apply_relationship(txn: &mut Txn, a: &str, b: &str, delta: i64) -> Result<(), Reason> {
    if !(-1..=1).contains(&delta) {
        return Err(Reason::OutOfRange);
    }
    let world = txn.now().world_time;
    let a_busy = txn.now().actors.get(a).ok_or(Reason::UnknownKey)?.busy();
    let b_busy = txn.now().actors.get(b).ok_or(Reason::UnknownKey)?.busy();
    let relation = txn
        .now()
        .actors
        .get(a)
        .and_then(|actor| actor.relationships())
        .and_then(|relations| relations.get(b))
        .ok_or(Reason::UnknownKey)?;
    if a_busy
        || b_busy
        || world.saturating_sub(relation.last_interaction_world_time) < MINUTES_PER_DAY
    {
        return Err(Reason::Skipped);
    }
    let current_stance = relation.stance;
    let current_at = relation.last_interaction_world_time;
    let next_stance = current_stance
        .checked_add(delta)
        .ok_or(Reason::OutOfRange)?;
    if next_stance != current_stance || world != current_at {
        txn.track(StateKey::Relationship(a.to_string(), b.to_string()))?;
    }
    let relation = txn
        .now_mut()
        .actors
        .get_mut(a)
        .and_then(|actor| actor.relationships_mut())
        .and_then(|relations| relations.get_mut(b))
        .ok_or(Reason::UnknownKey)?;
    relation.stance = next_stance;
    relation.last_interaction_world_time = world;
    Ok(())
}

fn apply_reputation(
    txn: &mut Txn,
    actor_id: &str,
    faction: &str,
    delta: i64,
) -> Result<(), Reason> {
    if !(-1..=1).contains(&delta) {
        return Err(Reason::OutOfRange);
    }
    let world = txn.now().world_time;
    let current = txn
        .now()
        .actors
        .get(actor_id)
        .and_then(|actor| match actor {
            crate::model::ActorLive::Npc { reputation, .. } => reputation.get(faction),
            crate::model::ActorLive::Pc { .. } => None,
        });
    if let Some(rep) = current {
        if world.saturating_sub(rep.last_world_time) < MINUTES_PER_DAY {
            return Err(Reason::Skipped);
        }
    }
    let current_value = current.map(|rep| rep.value).unwrap_or(0);
    let current_at = current.map(|rep| rep.last_world_time);
    let next = current_value.checked_add(delta).ok_or(Reason::OutOfRange)?;
    if next != current_value || current_at != Some(world) {
        txn.track(StateKey::Reputation(
            actor_id.to_string(),
            faction.to_string(),
        ))?;
    }
    let reputation = txn
        .now_mut()
        .actors
        .get_mut(actor_id)
        .and_then(|actor| actor.reputation_mut())
        .ok_or(Reason::UnknownKey)?;
    reputation.insert(
        faction.to_string(),
        Reputation {
            value: next,
            last_world_time: world,
        },
    );
    Ok(())
}

fn apply_memory(
    txn: &mut Txn,
    intent: &Intent,
    actor_id: &str,
    fact_ids: Vec<String>,
    tags: Vec<String>,
    text: Option<String>,
) -> Result<(), Reason> {
    if actor_id != intent.actor_id {
        return Err(Reason::ActorMismatch);
    }
    if text.as_ref().is_some_and(|text| text.chars().count() > 240) {
        return Err(Reason::Schema);
    }
    let viewer = match intent.actor_kind {
        ActorKind::Npc => ViewerId::Npc(actor_id.to_string()),
        ActorKind::Player => ViewerId::Player(actor_id.to_string()),
        ActorKind::System => return Err(Reason::ActorMismatch),
    };
    for fact in &fact_ids {
        if !allows(txn.canon(), txn.now(), &viewer, fact) {
            return Err(Reason::IllegalKnowledge);
        }
    }
    if let Some(text) = &text {
        let leaked = txn.canon().facts.iter().any(|(id, fact)| {
            !fact.text.is_empty()
                && text.contains(&fact.text)
                && !allows(txn.canon(), txn.now(), &viewer, id)
        });
        if leaked {
            return Err(Reason::IllegalKnowledge);
        }
    }
    txn.bump_memory()?;
    let mut fact_ids = fact_ids;
    fact_ids.sort();
    fact_ids.dedup();
    let mut tags = tags;
    tags.sort();
    tags.dedup();
    txn.stage_memory(MemoryLine {
        tick_index: txn.now().tick_index,
        actor_id: actor_id.to_string(),
        intent_id: intent.id.clone(),
        fact_ids,
        tags,
        text,
    });
    Ok(())
}

fn apply_rumor(
    txn: &mut Txn,
    rumor_id: &str,
    fact_ids: &[String],
    to: &str,
    heat: i64,
) -> Result<(), Reason> {
    if !(1..=3).contains(&heat) {
        return Err(Reason::OutOfRange);
    }
    let kind = match txn.now().actors.get(to) {
        Some(actor) if actor.is_npc() => ActorKind::Npc,
        Some(actor) if actor.is_pc() => ActorKind::Player,
        _ => return Err(Reason::UnknownKey),
    };
    let viewer = ViewerId::from_kind(kind, to)?;
    for fact in fact_ids {
        if !allows(txn.canon(), txn.now(), &viewer, fact) {
            return Err(Reason::IllegalKnowledge);
        }
    }
    let known = txn
        .now()
        .rumors
        .iter()
        .find(|rumor| rumor.id == rumor_id)
        .ok_or(Reason::UnknownKey)?
        .known_by
        .clone();
    if known.iter().any(|id| id == to) {
        return Ok(());
    }
    txn.bump_rumor()?;
    let mut next = known;
    next.push(to.to_string());
    next.sort();
    next.dedup();
    txn.track(StateKey::Rumor(rumor_id.to_string()))?;
    let rumor = txn
        .now_mut()
        .rumors
        .iter_mut()
        .find(|rumor| rumor.id == rumor_id)
        .ok_or(Reason::UnknownKey)?;
    rumor.known_by = next;
    Ok(())
}

fn apply_resource(txn: &mut Txn, actor_id: &str, key: &str, delta: i64) -> Result<(), Reason> {
    let actor = txn.now().actors.get(actor_id).ok_or(Reason::UnknownKey)?;
    let current = match actor {
        crate::model::ActorLive::Npc { resources, .. } => {
            if !txn
                .canon()
                .profile(actor_id)
                .is_some_and(|profile| profile.resources.contains_key(key))
            {
                return Err(Reason::UnknownKey);
            }
            resources.get(key).copied().unwrap_or(0)
        }
        crate::model::ActorLive::Pc { .. } => return Err(Reason::NotInSpec),
    };
    let next = resource_next(current, delta)?;
    if next != current {
        txn.track(StateKey::ActorResource(
            actor_id.to_string(),
            key.to_string(),
        ))?;
    }
    match txn.now_mut().actors.get_mut(actor_id) {
        Some(crate::model::ActorLive::Npc { resources, .. }) => {
            resources.insert(key.to_string(), next);
        }
        _ => return Err(Reason::UnknownKey),
    }
    Ok(())
}

fn apply_flag(txn: &mut Txn, actor_id: &str, key: &str, value: bool) -> Result<(), Reason> {
    if key != "busy" {
        let listed = txn
            .canon()
            .profile(actor_id)
            .is_some_and(|profile| profile.flags.iter().any(|flag| flag == key));
        if !listed {
            return Err(Reason::UnknownKey);
        }
    }
    if key == "busy" {
        let current = txn.now().actors.get(actor_id).map(|actor| actor.busy());
        if current != Some(value) {
            txn.track(StateKey::ActorBusy(actor_id.to_string()))?;
        }
        *txn.now_mut()
            .actors
            .get_mut(actor_id)
            .ok_or(Reason::UnknownKey)?
            .busy_mut() = value;
        return Ok(());
    }
    let has = txn
        .now()
        .actors
        .get(actor_id)
        .is_some_and(|actor| match actor {
            crate::model::ActorLive::Npc { flags, .. }
            | crate::model::ActorLive::Pc { flags, .. } => flags.iter().any(|flag| flag == key),
        });
    if has != value {
        txn.track(StateKey::ActorFlag(actor_id.to_string(), key.to_string()))?;
    }
    let flags = txn
        .now_mut()
        .actors
        .get_mut(actor_id)
        .ok_or(Reason::UnknownKey)?
        .flags_mut();
    if value {
        if !flags.iter().any(|flag| flag == key) {
            flags.push(key.to_string());
        }
    } else {
        flags.retain(|flag| flag != key);
    }
    Ok(())
}

fn apply_proposal(
    txn: &mut Txn,
    intent: &Intent,
    proposal_id: &str,
    kind: &str,
    target: &str,
    summary: &str,
) -> Result<(), Reason> {
    if !matches!(kind, "delete_place" | "patch_entity" | "other") {
        return Err(Reason::Schema);
    }
    if proposal_id.is_empty() || proposal_id.contains('/') || proposal_id.contains('\\') {
        return Err(Reason::Schema);
    }
    if txn.now().proposals.iter().any(|id| id == proposal_id) {
        return Err(Reason::UnknownKey);
    }
    let mut next = txn.now().proposals.clone();
    next.push(proposal_id.to_string());
    next.sort();
    next.dedup();
    let changed = next != txn.now().proposals;
    if changed {
        txn.track(StateKey::Proposal(proposal_id.to_string()))?;
    }
    txn.now_mut().proposals = next;
    txn.stage_proposal(ProposalDoc {
        schema: "proposal/v0".to_string(),
        id: proposal_id.to_string(),
        kind: kind.to_string(),
        target: target.to_string(),
        summary: summary.to_string(),
        actor_id: intent.actor_id.clone(),
        source: intent.source.token().to_string(),
        tick_index: intent.tick_index,
    });
    Ok(())
}

fn apply_message(
    txn: &mut Txn,
    message_id: String,
    to: Vec<String>,
    place: Option<String>,
    fact_ids: Vec<String>,
    text: String,
) -> Result<(), Reason> {
    if txn
        .now()
        .messages
        .iter()
        .any(|message| message.id == message_id)
    {
        return Err(Reason::UnknownKey);
    }
    if let Some(place_id) = &place {
        if !txn.now().places.contains_key(place_id) {
            return Err(Reason::UnknownKey);
        }
    }
    txn.track(StateKey::Message(message_id.clone()))?;
    if let Some(place_id) = &place {
        let mut notes = txn
            .now()
            .places
            .get(place_id)
            .map(|place_live| place_live.notes.clone())
            .unwrap_or_default();
        notes.push(message_id.clone());
        notes.sort();
        notes.dedup();
        if let Some(place_live) = txn.now_mut().places.get_mut(place_id) {
            place_live.notes = notes;
        }
    }
    let mut to = to;
    to.sort();
    to.dedup();
    let mut fact_ids = fact_ids;
    fact_ids.sort();
    fact_ids.dedup();
    txn.now_mut().messages.push(Message {
        id: message_id,
        to,
        place,
        fact_ids,
        text,
    });
    Ok(())
}
