use crate::effect::{Effect, EffectClass};
use crate::error::Reason;
use crate::firewall::allows;
use crate::intent::{ActorKind, Intent, ViewerId};
use crate::key::StateKey;
use crate::meter::{self, MeterWrite};
use crate::model::{MemoryLine, Message};
use crate::world::Draft;

pub fn apply_effect(draft: &mut Draft, intent: &Intent, effect: Effect) -> Result<(), Reason> {
    if !viewer_matches(intent, &effect) {
        return Err(Reason::ViewerMismatch);
    }
    match effect.class {
        EffectClass::Clock { clock_id, delta } => {
            apply_clock(draft, &intent.actor_id, &clock_id, delta)
        }
        EffectClass::Meter {
            actor_id,
            key,
            write,
        } => apply_meter(draft, &actor_id, &key, write),
        EffectClass::Stock { place, sku, delta } => apply_stock(draft, intent, &place, &sku, delta),
        EffectClass::Location { actor_id, from, to } => {
            apply_location(draft, intent, &actor_id, &from, &to)
        }
        EffectClass::Memory {
            actor_id,
            fact_ids,
            tags,
            text,
        } => apply_memory(draft, intent, &actor_id, fact_ids, tags, text),
        EffectClass::Flag {
            actor_id,
            key,
            value,
        } => apply_flag(draft, &actor_id, &key, value),
        EffectClass::Message {
            message_id,
            to,
            place,
            fact_ids,
            text,
        } => apply_message(draft, message_id, to, place, fact_ids, text),
    }
}

fn viewer_matches(intent: &Intent, effect: &Effect) -> bool {
    match (intent.actor_kind, effect.as_viewer()) {
        (ActorKind::Npc, ViewerId::Npc(id)) => id == &intent.actor_id,
        (ActorKind::Player, ViewerId::Player(id)) => id == &intent.actor_id,
        _ => false,
    }
}

fn apply_clock(
    draft: &mut Draft,
    actor_id: &str,
    clock_id: &str,
    delta: i64,
) -> Result<(), Reason> {
    let humans = draft.now().presence.humans;
    let clock = draft.now().clocks.get(clock_id).ok_or(Reason::UnknownKey)?;
    if !clock.owners.iter().any(|owner| owner == actor_id) {
        return Err(Reason::UnknownKey);
    }
    if clock.paused || (clock.requires_presence && humans == 0) {
        return Err(Reason::PausedClock);
    }
    let current = clock.value;
    let next = current.checked_add(delta).ok_or(Reason::OutOfRange)?;
    if next < 0 || next > clock.max {
        return Err(Reason::OutOfRange);
    }
    if next != current {
        draft.track(StateKey::ClockValue(clock_id.to_string()))?;
    }
    draft
        .now_mut()
        .clocks
        .get_mut(clock_id)
        .ok_or(Reason::UnknownKey)?
        .value = next;
    Ok(())
}

fn apply_meter(
    draft: &mut Draft,
    actor_id: &str,
    key: &str,
    write: MeterWrite,
) -> Result<(), Reason> {
    let def = *draft.canon().meters.get(key).ok_or(Reason::UnknownKey)?;
    let mut live = draft
        .now()
        .actors
        .get(actor_id)
        .ok_or(Reason::UnknownKey)?
        .meters()
        .get(key)
        .cloned()
        .ok_or(Reason::UnknownKey)?;
    let changed = meter::write(&def, &mut live, write)?;
    if changed {
        draft.track(StateKey::ActorMeter(actor_id.to_string(), key.to_string()))?;
    }
    *draft
        .now_mut()
        .actors
        .get_mut(actor_id)
        .ok_or(Reason::UnknownKey)?
        .meters_mut()
        .get_mut(key)
        .ok_or(Reason::UnknownKey)? = live;
    Ok(())
}

fn stock_next(current: i64, delta: i64) -> Result<i64, Reason> {
    let next = current.checked_add(delta).ok_or(Reason::OutOfRange)?;
    if next < 0 {
        return Err(Reason::OutOfRange);
    }
    Ok(next)
}

fn apply_stock(
    draft: &mut Draft,
    intent: &Intent,
    place: &str,
    sku: &str,
    delta: i64,
) -> Result<(), Reason> {
    let profile = draft
        .canon()
        .profile(&intent.actor_id)
        .ok_or(Reason::NoProfile)?;
    if !profile.range.iter().any(|item| item == place) {
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
    let current = draft
        .now()
        .places
        .get(place)
        .and_then(|place_live| place_live.stock.get(sku))
        .copied()
        .unwrap_or(0);
    let next = stock_next(current, delta)?;
    if next != current {
        draft.track(StateKey::PlaceStock(place.to_string(), sku.to_string()))?;
    }
    let place_live = draft
        .now_mut()
        .places
        .get_mut(place)
        .ok_or(Reason::OutOfRange)?;
    place_live.stock.insert(sku.to_string(), next);
    Ok(())
}

fn apply_location(
    draft: &mut Draft,
    intent: &Intent,
    actor_id: &str,
    from: &str,
    to: &str,
) -> Result<(), Reason> {
    if actor_id != intent.actor_id {
        return Err(Reason::ActorMismatch);
    }
    let actor = draft.now().actors.get(actor_id).ok_or(Reason::UnknownKey)?;
    if draft.now().presence.humans == 0 && actor.is_pc() {
        return Err(Reason::TableEmpty);
    }
    if actor.place() != from {
        return Err(Reason::OutOfRange);
    }
    let profile = draft.canon().profile(actor_id).ok_or(Reason::NoProfile)?;
    if !profile.range.iter().any(|item| item == to) {
        return Err(Reason::OutOfRange);
    }
    let place_changed = actor.place() != to;
    if place_changed {
        draft.track(StateKey::ActorPlace(actor_id.to_string()))?;
    }
    *draft
        .now_mut()
        .actors
        .get_mut(actor_id)
        .ok_or(Reason::UnknownKey)?
        .place_mut() = to.to_string();
    Ok(())
}

fn apply_memory(
    draft: &mut Draft,
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
        if !allows(draft.canon(), draft.now(), &viewer, fact) {
            return Err(Reason::IllegalKnowledge);
        }
    }
    if let Some(text) = &text {
        let leaked = draft.canon().facts.iter().any(|(id, fact)| {
            !fact.text.is_empty()
                && text.contains(&fact.text)
                && !allows(draft.canon(), draft.now(), &viewer, id)
        });
        if leaked {
            return Err(Reason::IllegalKnowledge);
        }
    }
    draft.bump_memory()?;
    let mut fact_ids = fact_ids;
    fact_ids.sort();
    fact_ids.dedup();
    let mut tags = tags;
    tags.sort();
    tags.dedup();
    draft.stage_memory(MemoryLine {
        tick_index: draft.now().tick_index,
        actor_id: actor_id.to_string(),
        intent_id: intent.id.clone(),
        fact_ids,
        tags,
        text,
    });
    Ok(())
}

fn apply_flag(draft: &mut Draft, actor_id: &str, key: &str, value: bool) -> Result<(), Reason> {
    if key != "busy" {
        let listed = draft
            .canon()
            .profile(actor_id)
            .is_some_and(|profile| profile.flags.iter().any(|flag| flag == key));
        if !listed {
            return Err(Reason::UnknownKey);
        }
    }
    if key == "busy" {
        let current = draft.now().actors.get(actor_id).map(|actor| actor.busy());
        if current != Some(value) {
            draft.track(StateKey::ActorBusy(actor_id.to_string()))?;
        }
        *draft
            .now_mut()
            .actors
            .get_mut(actor_id)
            .ok_or(Reason::UnknownKey)?
            .busy_mut() = value;
        return Ok(());
    }
    let has = draft
        .now()
        .actors
        .get(actor_id)
        .is_some_and(|actor| match actor {
            crate::model::ActorLive::Npc { flags, .. }
            | crate::model::ActorLive::Pc { flags, .. } => flags.iter().any(|flag| flag == key),
        });
    if has != value {
        draft.track(StateKey::ActorFlag(actor_id.to_string(), key.to_string()))?;
    }
    let flags = draft
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

fn apply_message(
    draft: &mut Draft,
    message_id: String,
    to: Vec<String>,
    place: Option<String>,
    fact_ids: Vec<String>,
    text: String,
) -> Result<(), Reason> {
    if draft
        .now()
        .messages
        .iter()
        .any(|message| message.id == message_id)
    {
        return Err(Reason::UnknownKey);
    }
    if let Some(place_id) = &place {
        if !draft.now().places.contains_key(place_id) {
            return Err(Reason::UnknownKey);
        }
    }
    draft.track(StateKey::Message(message_id.clone()))?;
    if let Some(place_id) = &place {
        let mut notes = draft
            .now()
            .places
            .get(place_id)
            .map(|place_live| place_live.notes.clone())
            .unwrap_or_default();
        notes.push(message_id.clone());
        notes.sort();
        notes.dedup();
        if let Some(place_live) = draft.now_mut().places.get_mut(place_id) {
            place_live.notes = notes;
        }
    }
    let mut to = to;
    to.sort();
    to.dedup();
    let mut fact_ids = fact_ids;
    fact_ids.sort();
    fact_ids.dedup();
    draft.now_mut().messages.push(Message {
        id: message_id,
        to,
        place,
        fact_ids,
        text,
    });
    Ok(())
}
