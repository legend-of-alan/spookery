use std::collections::BTreeSet;

use crate::canon::Canon;
use crate::error::Reason;
use crate::firewall::allows;
use crate::intent::ViewerId;
use crate::model::{Message, Now};
use crate::shard::Shard;

const BUDGET_TOKENS: usize = 800;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Perceive {
    pub viewer: String,
    pub place: String,
    pub visible_actors: Vec<String>,
    pub known_facts: Vec<KnownFact>,
    pub messages: Vec<Message>,
    pub unknown_mentioned: Vec<String>,
    pub clocks: Vec<ClockView>,
    pub stocks: Vec<StockView>,
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
pub struct StockView {
    pub sku: String,
    pub qty: i64,
}

pub fn perceive(shard: &Shard, as_arg: &str) -> Result<Perceive, Reason> {
    perceive_budget(shard, as_arg, BUDGET_TOKENS)
}

pub fn perceive_budget(
    shard: &Shard,
    as_arg: &str,
    budget_tokens: usize,
) -> Result<Perceive, Reason> {
    perceive_at_budget(shard.canon(), shard.now(), as_arg, budget_tokens)
}

pub fn perceive_at(canon: &Canon, now: &Now, as_arg: &str) -> Result<Perceive, Reason> {
    perceive_at_budget(canon, now, as_arg, BUDGET_TOKENS)
}

fn perceive_at_budget(
    canon: &Canon,
    now: &Now,
    as_arg: &str,
    budget_tokens: usize,
) -> Result<Perceive, Reason> {
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

    let stocks = now
        .places
        .get(&place)
        .map(|place_live| {
            place_live
                .stock
                .iter()
                .map(|(sku, qty)| StockView {
                    sku: sku.clone(),
                    qty: *qty,
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(Perceive {
        viewer: viewer.token(),
        place,
        visible_actors,
        known_facts,
        messages,
        unknown_mentioned,
        clocks,
        stocks,
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
