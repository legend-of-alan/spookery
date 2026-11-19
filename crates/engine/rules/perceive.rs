//! What one viewer is shown, trimmed to the prose budget.

use std::collections::BTreeSet;

use crate::canon::Canon;
use crate::ecs::component::Meter;
use crate::ecs::component::Note;
use crate::ecs::id::{ActorId, GlobalId, PlaceId};
use crate::ecs::{self, component, Presence};
use crate::entity;
use crate::intent::Viewer;
use crate::rules::habits::list_whose_if_holds;
use crate::rules::visibility::can_see_fact;
use crate::session;
use crate::Reason;
use spookery_protocol::{ExitView, NoteView, PERCEIVE_BODY_CHARS};
pub(crate) use spookery_protocol::{ItemStack, KnownFact, MeterView};

/// What one viewer is shown. Returned from the engine to the host table, which renders it as
/// a player or GM view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View {
    pub viewer: Viewer,
    pub place: String,
    pub place_name: String,
    pub prose: String,
    pub visible_actors: Vec<ActorId>,
    pub actor_names: Vec<String>,
    pub known_facts: Vec<KnownFact>,
    pub notes: Vec<NoteView>,
    pub unknown_mentions: Vec<String>,
    /// The items lying in the viewer's place.
    pub items: Vec<ItemStack>,
    /// The items the viewer holds.
    pub carried: Vec<ItemStack>,
    /// Which of `carried` the viewer wields.
    pub wielded_item_id: Option<String>,
    /// The viewer's own meters. A hidden meter is not here.
    pub meters: Vec<MeterView>,
    /// The world's meters. A hidden one is not here.
    pub world_meters: Vec<MeterView>,
    pub habits: Vec<String>,
    pub exits: Vec<ExitView>,
    /// What keeps the viewer busy and until when, or empty when they are free.
    pub busy: String,
}

pub(crate) fn build_view_by_id<Lock>(
    session: &session::Session<Lock>,
    presence: &Presence,
    viewer_id: &str,
    max_body_chars: usize,
) -> Result<View, Reason> {
    if viewer_id.is_empty()
        || viewer_id == "gm"
        || viewer_id.contains(':')
        || viewer_id.chars().any(char::is_whitespace)
    {
        return Err(Reason::UnscopedPerceive);
    }
    let (canon, world) = (session.canon(), session.world());
    let id = ActorId::from(viewer_id);
    let control = world.actors().get::<component::ControlType>(&id);
    let viewer = control.viewer().ok_or(Reason::UnknownId)?;
    build_capped_view(canon, world, presence, &viewer, max_body_chars)
}

pub(crate) fn build_view(
    canon: &Canon,
    world: &ecs::World,
    presence: &Presence,
    viewer: &Viewer,
) -> Result<View, Reason> {
    build_capped_view(canon, world, presence, viewer, PERCEIVE_BODY_CHARS)
}

fn build_capped_view(
    canon: &Canon,
    world: &ecs::World,
    presence: &Presence,
    viewer: &Viewer,
    max_body_chars: usize,
) -> Result<View, Reason> {
    let place_id = PlaceId::from(
        world
            .actors()
            .get::<component::Place>(viewer.actor_id())
            .get()
            .ok_or(Reason::UnknownId)?,
    );
    let mut visible_actors: Vec<ActorId> = world.actors_at(place_id.as_str());
    visible_actors.sort();

    let mut ids = BTreeSet::new();
    for id in canon.facts.keys() {
        ids.insert(id.clone());
    }
    for id in world
        .actors()
        .get::<component::KnownFacts>(viewer.actor_id())
        .get()
        .into_iter()
        .flatten()
    {
        ids.insert(id.clone());
    }
    let mut known_facts = Vec::new();
    let mut public_flags = Vec::new();
    for id in ids {
        if !can_see_fact(canon, world, viewer, &id) {
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
    for (note_id, note) in world.globals().get::<Note>(&GlobalId).iter() {
        let here = note.place_id.as_ref() == Some(&place_id);
        let addressed = note.to_ids.contains(viewer.actor_id());
        if !here && !addressed {
            continue;
        }
        if note
            .fact_ids
            .iter()
            .any(|fact| !can_see_fact(canon, world, viewer, fact))
        {
            continue;
        }
        notes.push(NoteView {
            id: note_id.to_string(),
            to_ids: note.to_ids.iter().map(ActorId::to_string).collect(),
            place_id: note.place_id.as_ref().map(PlaceId::to_string),
            fact_ids: note.fact_ids.iter().cloned().collect(),
            text: note.text.clone(),
        });
    }

    let (places, actors, me) = (world.places(), world.actors(), viewer.actor_id());
    let items = item_stacks(canon, places.get::<component::Item>(&place_id).iter())?;
    let carried = item_stacks(canon, actors.get::<component::Item>(me).iter())?;
    let wielded_item_id = (actors.get::<component::WieldedItem>(me).get()).map(str::to_string);

    let meters = shown_meters(canon, world, &entity::Id::Actor(viewer.actor_id().clone()));
    let world_meters = shown_meters(canon, world, &entity::Id::Global);

    let habits = match viewer {
        Viewer::Player(_) => Vec::new(),
        Viewer::Npc(id) => list_whose_if_holds(canon, world, presence, id),
    };
    let exits = crate::rules::travel::visible_exits(canon, world, viewer.actor_id());
    let actor_names = visible_actors
        .iter()
        .map(|id| canon.name(id.as_str()).map(str::to_string))
        .collect::<Option<_>>()
        .ok_or(Reason::UnknownId)?;
    let place_name =
        crate::rules::lines::find_place_name(canon, place_id.as_str()).ok_or(Reason::UnknownId)?;
    let prose = crate::rules::lines::place_prose(canon, world, viewer.actor_id());
    let busy = crate::rules::lines::busy_line(canon, world, viewer.actor_id()).unwrap_or_default();

    Ok(View {
        viewer: viewer.clone(),
        place: place_id.into(),
        place_name,
        prose,
        visible_actors,
        actor_names,
        known_facts,
        notes,
        unknown_mentions,
        items,
        carried,
        wielded_item_id,
        meters,
        world_meters,
        habits,
        exits,
        busy,
    })
}

/// Each item of `held`, with its count and name.
fn item_stacks<'a>(
    canon: &Canon,
    held: impl Iterator<Item = (&'a str, &'a u32)>,
) -> Result<Vec<ItemStack>, Reason> {
    held.map(|(id, count)| {
        Ok(ItemStack {
            id: id.to_string(),
            count: *count,
            name: canon.name(id).ok_or(Reason::UnknownId)?.to_string(),
        })
    })
    .collect()
}

/// The meters `carrier` carries that are not hidden, in id order.
fn shown_meters(canon: &Canon, world: &ecs::World, carrier: &entity::Id) -> Vec<MeterView> {
    let descs = if *carrier == entity::Id::Global {
        &canon.world_meters
    } else {
        &canon.meters
    };
    descs
        .keys()
        .filter_map(|id| {
            let desc = canon
                .meter_desc(carrier.as_ref(), id)
                .filter(|desc| !desc.hidden)?;
            let value = world.entities().get::<Meter>(carrier).value(id).ok()?;
            Some(MeterView {
                id: id.to_string(),
                name: desc.name.clone(),
                value,
                max: desc.max,
            })
        })
        .collect()
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
