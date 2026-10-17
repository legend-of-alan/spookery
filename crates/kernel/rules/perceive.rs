//! What one viewer is shown, trimmed to the prose budget.

use std::collections::BTreeSet;

use crate::canon::Canon;
use crate::error::Reason;
use crate::firewall::allows;
use crate::intent::ViewerId;
use crate::map::ExitView;
use crate::meter;
use crate::model::{Message, Now};
use crate::rules::habits::habits_on;
use crate::world::World;

const BUDGET_TOKENS: usize = 800;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View {
    pub viewer: String,
    pub place: String,
    pub place_name: String,
    pub prose: String,
    pub visible_actors: Vec<String>,
    pub actor_names: Vec<String>,
    pub known_facts: Vec<KnownFact>,
    pub messages: Vec<Message>,
    pub unknown_mentioned: Vec<String>,
    pub clocks: Vec<ClockView>,
    pub items: Vec<ItemStack>,
    pub meters: Vec<MeterView>,
    pub habits: Vec<String>,
    pub exits: Vec<ExitView>,
    pub status: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KnownFact {
    pub id: String,
    pub text: String,
    pub body: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClockView {
    pub id: String,
    pub value: i64,
    pub max: i64,
    pub paused: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemStack {
    pub id: String,
    pub count: u32,
}

/// One of the viewer's own meters. A hidden meter is not here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MeterView {
    pub id: String,
    pub value: i64,
    pub max: i64,
}

pub fn perceive(world: &World, as_arg: &str) -> Result<View, Reason> {
    perceive_budget(world, as_arg, BUDGET_TOKENS)
}

pub fn perceive_budget(world: &World, as_arg: &str, budget_tokens: usize) -> Result<View, Reason> {
    perceive_at_budget(world.canon(), world.now(), as_arg, budget_tokens)
}

pub fn perceive_at(canon: &Canon, now: &Now, as_arg: &str) -> Result<View, Reason> {
    perceive_at_budget(canon, now, as_arg, BUDGET_TOKENS)
}

fn perceive_at_budget(
    canon: &Canon,
    now: &Now,
    as_arg: &str,
    budget_tokens: usize,
) -> Result<View, Reason> {
    let viewer = ViewerId::parse(as_arg)?;
    let actor = now
        .actors
        .get(viewer.actor_id())
        .ok_or(Reason::UnknownKey)?;
    let place = actor.place().to_string();
    let mut visible_actors: Vec<String> = now
        .actors
        .iter()
        .filter(|(_, other)| other.place() == place)
        .map(|(id, _)| id.clone())
        .collect();
    visible_actors.sort();

    let mut ids = BTreeSet::new();
    for id in canon.facts.keys() {
        ids.insert(id.clone());
    }
    for id in actor.known_facts() {
        ids.insert(id.clone());
    }
    let mut known_facts = Vec::new();
    let mut public_flags = Vec::new();
    for id in ids {
        if !allows(canon, now, &viewer, &id) {
            continue;
        }
        let text = canon
            .facts
            .get(&id)
            .map(|fact| fact.text.clone())
            .unwrap_or_default();
        let body = canon.body_of_fact(&id).to_string();
        let public = canon.facts.get(&id).is_some_and(|fact| !fact.hidden);
        known_facts.push(KnownFact { id, text, body });
        public_flags.push(public);
    }

    let mut unknown_mentioned = Vec::new();
    cap_bodies(
        &mut known_facts,
        &public_flags,
        &mut unknown_mentioned,
        budget_tokens,
    );
    unknown_mentioned.sort();

    let mut messages = Vec::new();
    for message in &now.messages {
        let here = message.place.as_deref() == Some(place.as_str());
        let addressed = message.to.iter().any(|id| id == viewer.actor_id());
        if !here && !addressed {
            continue;
        }
        if message
            .fact_ids
            .iter()
            .any(|fact| !allows(canon, now, &viewer, fact))
        {
            continue;
        }
        if text_names_unallowed_fact(canon, now, &viewer, &message.text) {
            continue;
        }
        let mut copy = message.clone();
        copy.to.sort();
        copy.fact_ids.sort();
        messages.push(copy);
    }
    messages.sort_by(|a, b| a.id.cmp(&b.id));

    let mut clocks: Vec<ClockView> = now
        .clocks
        .iter()
        .filter(|(_, clock)| clock.owners.iter().any(|id| id == viewer.actor_id()))
        .map(|(id, clock)| ClockView {
            id: id.clone(),
            value: clock.value,
            max: clock.max,
            paused: clock.paused,
        })
        .collect();
    clocks.sort_by(|a, b| a.id.cmp(&b.id));

    let items = now
        .places
        .get(&place)
        .map(|place_live| {
            place_live
                .items
                .iter()
                .map(|(id, count)| ItemStack {
                    id: id.clone(),
                    count: *count,
                })
                .collect()
        })
        .unwrap_or_default();

    let meters = actor
        .meters()
        .iter()
        .filter_map(|(id, live)| {
            let def = canon.meters.get(id).filter(|def| !def.hidden)?;
            let value = meter::current(def, live).ok()?;
            Some(MeterView {
                id: id.clone(),
                value,
                max: def.max,
            })
        })
        .collect();

    let habits = match &viewer {
        ViewerId::Player(_) => Vec::new(),
        ViewerId::Npc(id) => habits_on(canon, now, id),
    };
    let exits = crate::map::visible(canon, now, viewer.actor_id());
    let actor_names = visible_actors
        .iter()
        .map(|id| crate::map::cast_name(canon, id))
        .collect();
    let place_name = crate::map::place_name(canon, &place);
    let prose = crate::map::place_prose(canon, now, viewer.actor_id());
    let status = crate::map::busy_line(now, viewer.actor_id()).unwrap_or_default();

    Ok(View {
        viewer: viewer.token(),
        place,
        place_name,
        prose,
        visible_actors,
        actor_names,
        known_facts,
        messages,
        unknown_mentioned,
        clocks,
        items,
        meters,
        habits,
        exits,
        status,
    })
}

/// A place note stays in the save. Who can read it changes as actors move,
/// so the drop is here and not in `apply`.
fn text_names_unallowed_fact(canon: &Canon, now: &Now, viewer: &ViewerId, text: &str) -> bool {
    canon.facts.iter().any(|(id, fact)| {
        !fact.text.is_empty()
            && text.contains(fact.text.as_str())
            && !allows(canon, now, viewer, id)
    })
}

fn cap_bodies(
    facts: &mut [KnownFact],
    public_flags: &[bool],
    unknown: &mut Vec<String>,
    budget_tokens: usize,
) {
    let mut sum = facts
        .iter()
        .map(|fact| fact.body.chars().count())
        .sum::<usize>();
    if sum <= budget_tokens {
        return;
    }
    let mut order: Vec<usize> = (0..facts.len()).collect();
    order.sort_by(|&left, &right| facts[right].id.cmp(&facts[left].id));
    for index in order {
        if sum <= budget_tokens {
            break;
        }
        let len = facts[index].body.chars().count();
        if len == 0 {
            continue;
        }
        let rest = sum - len;
        if len > budget_tokens && rest <= budget_tokens {
            let keep = budget_tokens - rest;
            facts[index].body = facts[index].body.chars().take(keep).collect();
            sum = budget_tokens;
        } else {
            facts[index].body.clear();
            sum = rest;
        }
        if public_flags.get(index).copied().unwrap_or(false) {
            unknown.push(facts[index].id.clone());
        }
    }
}
