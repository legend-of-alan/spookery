use crate::canon::Canon;
use crate::intent::ViewerId;
use crate::model::Now;

pub fn allows(canon: &Canon, now: &Now, viewer: &ViewerId, fact_id: &str) -> bool {
    let Some(actor) = now.actors.get(viewer.actor_id()) else {
        return false;
    };
    if actor.known_facts().iter().any(|id| id == fact_id) {
        return true;
    }
    let Some(fact) = canon.facts.get(fact_id) else {
        return false;
    };
    if fact.hidden {
        return false;
    }
    let place = actor.place();
    if canon
        .places
        .get(place)
        .is_some_and(|place_canon| place_canon.facts.iter().any(|id| id == fact_id))
    {
        return true;
    }
    now.actors
        .get(&fact.owner_id)
        .is_some_and(|other| other.place() == place)
        && canon
            .cast
            .get(&fact.owner_id)
            .is_some_and(|cast| cast.facts.iter().any(|id| id == fact_id))
}
