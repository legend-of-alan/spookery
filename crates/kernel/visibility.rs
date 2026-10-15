//! What one viewer may hear from a round.

use std::collections::BTreeSet;

use crate::canon::Canon;
use crate::effect::{Effect, EffectClass};
use crate::firewall::allows;
use crate::intent::{ActorKind, ViewerId};
use crate::key::{ChangeRecord, StateKey, StateValue};
use crate::model::Now;

/// Who is asking.
pub enum Seat {
    /// The referee sees every change.
    Gm,
    Actor(String),
}

/// The changes and narrate lines for one seat.
pub struct Delivery {
    pub changes: Vec<String>,
    pub lines: Vec<String>,
}

/// One narrate line addressed to one person.
pub struct Told {
    pub to: String,
    pub text: String,
}

/// The change keys one actor can see.
pub struct Seen {
    pub viewer: String,
    pub changes: Vec<String>,
}

/// Changes and narrate lines this seat can receive.
pub fn observe(
    canon: &Canon,
    now: &Now,
    seat: &Seat,
    changes: &[ChangeRecord],
    effects: &[Effect],
) -> Delivery {
    let actor = match seat {
        Seat::Gm => None,
        Seat::Actor(id) => Some(id.as_str()),
    };
    let mut keys = BTreeSet::new();
    for change in changes {
        if actor.is_none_or(|id| can_see(canon, now, id, change)) {
            keys.insert(change.key.to_string());
        }
    }
    let lines = match actor {
        None => narrations(canon, now, effects, None),
        Some(id) => narrations(canon, now, effects, Some(id)),
    };
    Delivery {
        changes: keys.into_iter().collect(),
        lines,
    }
}

fn narrations(canon: &Canon, now: &Now, effects: &[Effect], viewer: Option<&str>) -> Vec<String> {
    let mut lines = Vec::new();
    for effect in effects {
        let EffectClass::Narrate { to, fact_ids, text } = &effect.class else {
            continue;
        };
        let Some(id) = viewer else {
            lines.push(text.clone());
            continue;
        };
        if to.iter().any(|name| name == id) && narrate_allowed(canon, now, id, fact_ids, text) {
            lines.push(text.clone());
        }
    }
    lines
}

fn narrate_allowed(
    canon: &Canon,
    now: &Now,
    viewer: &str,
    fact_ids: &[String],
    text: &str,
) -> bool {
    let Some(seat) = viewer_of(now, viewer) else {
        return false;
    };
    if fact_ids.iter().any(|fact| !allows(canon, now, &seat, fact)) {
        return false;
    }
    !canon.facts.iter().any(|(fact_id, fact)| {
        !fact.text.is_empty() && text.contains(&fact.text) && !allows(canon, now, &seat, fact_id)
    })
}

fn can_see(canon: &Canon, now: &Now, viewer: &str, change: &ChangeRecord) -> bool {
    if !now.actors.contains_key(viewer) {
        return false;
    }
    match &change.key {
        StateKey::WorldFlag(_) => true,
        StateKey::TickLastAct(_) | StateKey::TickIdleActs(_) | StateKey::TickLiveActs(_) => false,
        StateKey::ActorKnownFact(actor) => actor == viewer,
        StateKey::PlaceItem(place, _) => here(now, viewer) == Some(place.as_str()),
        StateKey::Message(id) => message_visible(canon, now, viewer, id),
        StateKey::ClockValue(id) | StateKey::ClockPaused(id) => clock_owned(now, viewer, id),
        StateKey::ActorPlace(actor) => {
            actor == viewer
                || same_place(now, viewer, actor)
                || at(now, viewer, &change.before)
                || at(now, viewer, &change.after)
        }
        StateKey::ActorFlag(actor, _)
        | StateKey::ActorMeter(actor, _)
        | StateKey::ActorBusy(actor)
        | StateKey::ActorItem(actor, _)
        | StateKey::ActorWielded(actor) => actor == viewer || same_place(now, viewer, actor),
    }
}

fn here<'a>(now: &'a Now, viewer: &str) -> Option<&'a str> {
    now.actors.get(viewer).map(|actor| actor.place())
}

fn same_place(now: &Now, viewer: &str, actor: &str) -> bool {
    match (here(now, viewer), here(now, actor)) {
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}

fn at(now: &Now, viewer: &str, value: &StateValue) -> bool {
    match (here(now, viewer), value) {
        (Some(place), StateValue::Text(text)) => place == text,
        _ => false,
    }
}

fn clock_owned(now: &Now, viewer: &str, id: &str) -> bool {
    now.clocks
        .get(id)
        .is_some_and(|clock| clock.owners.iter().any(|owner| owner == viewer))
}

fn message_visible(canon: &Canon, now: &Now, viewer: &str, id: &str) -> bool {
    let Some(message) = now.messages.iter().find(|message| message.id == id) else {
        return false;
    };
    let in_room = message.place.as_deref() == here(now, viewer);
    let addressed = message.to.iter().any(|name| name == viewer);
    if !in_room && !addressed {
        return false;
    }
    let Some(seat) = viewer_of(now, viewer) else {
        return false;
    };
    if message
        .fact_ids
        .iter()
        .any(|fact| !allows(canon, now, &seat, fact))
    {
        return false;
    }
    !canon.facts.iter().any(|(fact_id, fact)| {
        !fact.text.is_empty()
            && message.text.contains(&fact.text)
            && !allows(canon, now, &seat, fact_id)
    })
}

fn viewer_of(now: &Now, id: &str) -> Option<ViewerId> {
    let actor = now.actors.get(id)?;
    let kind = if actor.is_pc() {
        ActorKind::Player
    } else {
        ActorKind::Npc
    };
    ViewerId::from_kind(kind, id).ok()
}
