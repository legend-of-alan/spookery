//! Write one effect into the staged save.

use crate::draft::Draft;
use crate::effect::{Effect, EffectKind, WORLD};
use crate::error::Reason;
use crate::intent::{ActorKind, Intent, ViewerId};
use crate::key::{StateKey, StateValue};
use crate::meter::{self, MeterWrite};
use crate::model::{self, MemoryLine, Message, PlaceLive};
use crate::visibility::allows;

pub fn apply_effect(draft: &mut Draft, intent: &Intent, effect: Effect) -> Result<(), Reason> {
    if !viewer_matches(intent, &effect) {
        return Err(Reason::ViewerMismatch);
    }
    match effect.class {
        EffectKind::Clock {
            clock_id,
            delta,
            start,
        } => apply_clock(draft, &intent.actor_id, &clock_id, delta, start),
        EffectKind::Meter {
            actor_id,
            key,
            write,
        } => apply_meter(draft, &actor_id, &key, write),
        EffectKind::Item {
            actor_id,
            place,
            id,
            delta,
            max,
        } => apply_item(draft, actor_id, place, &id, delta, max),
        EffectKind::Wield { actor_id, id } => apply_wield(draft, &actor_id, &id),
        EffectKind::Unwield { actor_id } => apply_unwield(draft, &actor_id),
        EffectKind::Location {
            actor_id, from, to, ..
        } => apply_location(draft, intent, &actor_id, &from, &to),
        EffectKind::Memory {
            actor_id,
            fact_ids,
            tags,
            text,
        } => apply_memory(draft, intent, &actor_id, fact_ids, tags, text),
        EffectKind::Flag {
            actor_id,
            key,
            value,
        } => apply_flag(draft, &actor_id, &key, value),
        EffectKind::Message {
            message_id,
            to,
            place,
            fact_ids,
            text,
        } => apply_message(draft, message_id, to, place, fact_ids, text),
        EffectKind::Narrate { .. } => Ok(()),
        EffectKind::Reveal { actor_id, fact } => apply_reveal(draft, &actor_id, &fact),
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
    start: bool,
) -> Result<(), Reason> {
    let humans = draft.now().presence.humans;
    let clock = draft.now().clocks.get(clock_id).ok_or(Reason::UnknownKey)?;
    if !clock.owners.iter().any(|owner| owner == actor_id) {
        return Err(Reason::UnknownKey);
    }
    if start && clock.paused {
        draft.track(StateKey::ClockPaused(clock_id.to_string()))?;
        draft
            .now_mut()
            .clocks
            .get_mut(clock_id)
            .ok_or(Reason::UnknownKey)?
            .paused = false;
    }
    let clock = draft.now().clocks.get(clock_id).ok_or(Reason::UnknownKey)?;
    if delta == 0 && start {
        return Ok(());
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

fn apply_item(
    draft: &mut Draft,
    actor_id: Option<String>,
    place: Option<String>,
    id: &str,
    delta: i64,
    max: Option<i64>,
) -> Result<(), Reason> {
    match (actor_id, place) {
        (Some(actor_id), None) => {
            let before = i64::from(
                draft
                    .now()
                    .actors
                    .get(&actor_id)
                    .ok_or(Reason::UnknownKey)?
                    .count(id),
            );
            let wielded = draft
                .now()
                .actors
                .get(&actor_id)
                .ok_or(Reason::UnknownKey)?
                .wielded()
                .map(str::to_string);
            let changed = {
                let inventory = draft
                    .now_mut()
                    .actors
                    .get_mut(&actor_id)
                    .ok_or(Reason::UnknownKey)?
                    .inventory_mut();
                model::shift_count(inventory, id, delta, max)?
            };
            if changed {
                draft.track_before(
                    StateKey::ActorItem(actor_id.clone(), id.to_string()),
                    StateValue::Number(before),
                )?;
            }
            let released = draft
                .now_mut()
                .actors
                .get_mut(&actor_id)
                .ok_or(Reason::UnknownKey)?
                .release_wielded();
            if released {
                draft.track_before(
                    StateKey::ActorWielded(actor_id),
                    wielded.map(StateValue::Text).unwrap_or(StateValue::Empty),
                )?;
            }
            Ok(())
        }
        (None, Some(place)) => {
            let before = i64::from(
                draft
                    .now()
                    .places
                    .get(&place)
                    .ok_or(Reason::UnknownKey)?
                    .items
                    .get(id)
                    .copied()
                    .unwrap_or(0),
            );
            let changed = {
                let items = &mut draft
                    .now_mut()
                    .places
                    .get_mut(&place)
                    .ok_or(Reason::UnknownKey)?
                    .items;
                model::shift_count(items, id, delta, max)?
            };
            if changed {
                draft.track_before(
                    StateKey::PlaceItem(place, id.to_string()),
                    StateValue::Number(before),
                )?;
            }
            Ok(())
        }
        _ => Err(Reason::Schema),
    }
}

fn apply_wield(draft: &mut Draft, actor_id: &str, item_id: &str) -> Result<(), Reason> {
    let Some(item) = draft.canon().items.get(item_id) else {
        return Err(Reason::UnknownKey);
    };
    if !item.tags.iter().any(|tag| tag == "weapon") {
        return Err(Reason::NotAWeapon);
    }
    let held = draft
        .now()
        .actors
        .get(actor_id)
        .ok_or(Reason::UnknownKey)?
        .count(item_id);
    if held == 0 {
        return Err(Reason::MissingItem);
    }
    let current = draft
        .now()
        .actors
        .get(actor_id)
        .ok_or(Reason::UnknownKey)?
        .wielded()
        .map(str::to_string);
    if current.as_deref() == Some(item_id) {
        return Ok(());
    }
    draft.track(StateKey::ActorWielded(actor_id.to_string()))?;
    *draft
        .now_mut()
        .actors
        .get_mut(actor_id)
        .ok_or(Reason::UnknownKey)?
        .wielded_mut() = Some(item_id.to_string());
    Ok(())
}

fn apply_unwield(draft: &mut Draft, actor_id: &str) -> Result<(), Reason> {
    let set = draft
        .now()
        .actors
        .get(actor_id)
        .ok_or(Reason::UnknownKey)?
        .wielded()
        .is_some();
    if !set {
        return Ok(());
    }
    draft.track(StateKey::ActorWielded(actor_id.to_string()))?;
    *draft
        .now_mut()
        .actors
        .get_mut(actor_id)
        .ok_or(Reason::UnknownKey)?
        .wielded_mut() = None;
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
    if !draft.canon().places.contains_key(to) && !draft.now().places.contains_key(to) {
        return Err(Reason::NoExit);
    }
    let place_changed = actor.place() != to;
    if place_changed {
        draft.track(StateKey::ActorPlace(actor_id.to_string()))?;
    }
    let actor = draft
        .now_mut()
        .actors
        .get_mut(actor_id)
        .ok_or(Reason::UnknownKey)?;
    *actor.place_mut() = to.to_string();
    actor.visit(to);
    if !draft.now().places.contains_key(to) {
        draft.now_mut().places.insert(
            to.to_string(),
            PlaceLive {
                items: std::collections::BTreeMap::new(),
                notes: Vec::new(),
                actors: Vec::new(),
            },
        );
    }
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
    draft.bump_memory()?;
    let mut fact_ids = fact_ids;
    fact_ids.sort();
    fact_ids.dedup();
    let known = draft
        .now()
        .actors
        .get(actor_id)
        .map(|actor| actor.known_facts().to_vec())
        .unwrap_or_default();
    if fact_ids
        .iter()
        .any(|fact| !known.iter().any(|id| id == fact))
    {
        draft.track(StateKey::ActorKnownFact(actor_id.to_string()))?;
        let facts = draft
            .now_mut()
            .actors
            .get_mut(actor_id)
            .ok_or(Reason::UnknownKey)?
            .known_facts_mut();
        for fact in &fact_ids {
            if !facts.iter().any(|id| id == fact) {
                facts.push(fact.clone());
            }
        }
        model::sort_unique(facts);
    }
    let mut tags = tags;
    tags.sort();
    tags.dedup();
    draft.stage_memory(MemoryLine {
        round_index: draft.now().round_index,
        actor_id: actor_id.to_string(),
        intent_id: intent.id.clone(),
        fact_ids,
        tags,
        text,
    });
    Ok(())
}

fn apply_flag(draft: &mut Draft, actor_id: &str, key: &str, value: bool) -> Result<(), Reason> {
    if actor_id == WORLD {
        return apply_world_flag(draft, key, value);
    }
    let listed = draft
        .canon()
        .profile(actor_id)
        .is_some_and(|profile| profile.flags.iter().any(|flag| flag == key));
    if !listed {
        return Err(Reason::UnknownKey);
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

fn apply_world_flag(draft: &mut Draft, key: &str, value: bool) -> Result<(), Reason> {
    if key.is_empty() || key == "busy" {
        return Err(Reason::UnknownKey);
    }
    let has = draft.now().flags.iter().any(|flag| flag == key);
    if has != value {
        draft.track(StateKey::WorldFlag(key.to_string()))?;
    }
    let flags = &mut draft.now_mut().flags;
    if value {
        if !flags.iter().any(|flag| flag == key) {
            flags.push(key.to_string());
        }
    } else {
        flags.retain(|flag| flag != key);
    }
    Ok(())
}

fn apply_reveal(draft: &mut Draft, actor_id: &str, fact: &str) -> Result<(), Reason> {
    if !draft.canon().facts.contains_key(fact) {
        return Err(Reason::UnknownKey);
    }
    let known = draft
        .now()
        .actors
        .get(actor_id)
        .ok_or(Reason::UnknownKey)?
        .known_facts();
    if known.iter().any(|id| id == fact) {
        return Ok(());
    }
    draft.track(StateKey::ActorKnownFact(actor_id.to_string()))?;
    let facts = draft
        .now_mut()
        .actors
        .get_mut(actor_id)
        .ok_or(Reason::UnknownKey)?
        .known_facts_mut();
    facts.push(fact.to_string());
    model::sort_unique(facts);
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
