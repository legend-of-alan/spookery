//! What one viewer is shown, trimmed to the prose budget.

use std::collections::BTreeSet;

use crate::canon::Canon;
use crate::error::Reason;
use crate::intent::Viewer;
use crate::meter;
use crate::model::{Note, Presence, State};
use crate::rules::habits::habits_that_hold;
use crate::travel::ExitView;
use crate::visibility::can_see_fact;
use crate::world::World;
pub use spookery_protocol::{ClockView, ItemStack, KnownFact, MeterView};

const MAX_BODY_CHARS: usize = 800;

/// Returned from the engine to the host table, which renders it as a player or GM view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View {
    pub viewer: Viewer,
    pub place: String,
    pub place_name: String,
    pub prose: String,
    pub visible_actors: Vec<String>,
    pub actor_names: Vec<String>,
    pub known_facts: Vec<KnownFact>,
    pub notes: Vec<Note>,
    pub unknown_mentions: Vec<String>,
    pub clocks: Vec<ClockView>,
    pub items: Vec<ItemStack>,
    /// The viewer's own meters. A hidden meter is not here.
    pub meters: Vec<MeterView>,
    pub habits: Vec<String>,
    pub exits: Vec<ExitView>,
    pub status: String,
}

pub fn perceive<L>(world: &World<L>, presence: &Presence, as_arg: &str) -> Result<View, Reason> {
    perceive_budget(world, presence, as_arg, MAX_BODY_CHARS)
}

pub fn perceive_budget<L>(
    world: &World<L>,
    presence: &Presence,
    as_arg: &str,
    max_body_chars: usize,
) -> Result<View, Reason> {
    if as_arg.is_empty()
        || as_arg == "gm"
        || as_arg.contains(':')
        || as_arg.chars().any(char::is_whitespace)
    {
        return Err(Reason::UnscopedPerceive);
    }
    let (canon, now) = (world.canon(), world.state());
    let actor = now.actors.get(as_arg).ok_or(Reason::UnknownId)?;
    perceive_viewer_budget(canon, now, presence, &actor.viewer(as_arg), max_body_chars)
}

pub(crate) fn perceive_as(
    canon: &Canon,
    now: &State,
    presence: &Presence,
    viewer: &Viewer,
) -> Result<View, Reason> {
    perceive_viewer_budget(canon, now, presence, viewer, MAX_BODY_CHARS)
}

fn perceive_viewer_budget(
    canon: &Canon,
    now: &State,
    presence: &Presence,
    viewer: &Viewer,
    max_body_chars: usize,
) -> Result<View, Reason> {
    let actor = now.actors.get(viewer.actor_id()).ok_or(Reason::UnknownId)?;
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
        if !can_see_fact(canon, now, viewer, &id) {
            continue;
        }
        let text = canon
            .facts
            .get(&id)
            .map(|fact| fact.text.clone())
            .unwrap_or_default();
        let body = canon.body_of_fact(&id).to_string();
        let public = canon.facts.get(&id).is_some_and(|fact| !fact.hidden);
        known_facts.push(KnownFact {
            id,
            text,
            owner_body: body,
        });
        public_flags.push(public);
    }

    let mut unknown_mentions = Vec::new();
    cap_bodies(
        &mut known_facts,
        &public_flags,
        &mut unknown_mentions,
        max_body_chars,
    );
    unknown_mentions.sort();

    let mut notes = Vec::new();
    for note in &now.notes {
        let here = note.place.as_deref() == Some(place.as_str());
        let addressed = note.to.iter().any(|id| id == viewer.actor_id());
        if !here && !addressed {
            continue;
        }
        if note
            .fact_ids
            .iter()
            .any(|fact| !can_see_fact(canon, now, viewer, fact))
        {
            continue;
        }
        let mut copy = note.clone();
        copy.to.sort();
        copy.fact_ids.sort();
        notes.push(copy);
    }
    notes.sort_by(|a, b| a.id.cmp(&b.id));

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

    let habits = match viewer {
        Viewer::Player(_) => Vec::new(),
        Viewer::Npc(id) => habits_that_hold(canon, now, presence, id),
    };
    let exits = crate::travel::visible_exits(canon, now, presence, viewer.actor_id());
    let actor_names = visible_actors
        .iter()
        .map(|id| canon.display_name(id))
        .collect();
    let place_name = crate::travel::place_name(canon, &place);
    let prose = crate::travel::place_prose(canon, now, viewer.actor_id());
    let status = crate::travel::busy_line(now, viewer.actor_id()).unwrap_or_default();

    Ok(View {
        viewer: viewer.clone(),
        place,
        place_name,
        prose,
        visible_actors,
        actor_names,
        known_facts,
        notes,
        unknown_mentions,
        clocks,
        items,
        meters,
        habits,
        exits,
        status,
    })
}

fn cap_bodies(
    facts: &mut [KnownFact],
    public_flags: &[bool],
    unknown: &mut Vec<String>,
    max_body_chars: usize,
) {
    let mut sum = facts
        .iter()
        .map(|fact| fact.owner_body.chars().count())
        .sum::<usize>();
    if sum <= max_body_chars {
        return;
    }
    let mut order: Vec<usize> = (0..facts.len()).collect();
    order.sort_by(|&left, &right| facts[right].id.cmp(&facts[left].id));
    for index in order {
        if sum <= max_body_chars {
            break;
        }
        let len = facts[index].owner_body.chars().count();
        if len == 0 {
            continue;
        }
        let rest = sum - len;
        if len > max_body_chars && rest <= max_body_chars {
            let keep = max_body_chars - rest;
            facts[index].owner_body = facts[index].owner_body.chars().take(keep).collect();
            sum = max_body_chars;
        } else {
            facts[index].owner_body.clear();
            sum = rest;
        }
        if public_flags.get(index).copied().unwrap_or(false) {
            unknown.push(facts[index].id.clone());
        }
    }
}
