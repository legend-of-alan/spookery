//! What one viewer is shown.

use crate::canon::Canon;
use crate::ecs::component::Meter;
use crate::ecs::component::Note;
use crate::ecs::entity::globals;
use crate::ecs::id::{ActorId, PlaceId};
use crate::ecs::{self, component, Presence};
use crate::entity;
use crate::rules::habits::list_whose_if_holds;
use crate::session;
use crate::Reason;
pub(crate) use spookery_protocol::{ItemStack, MeterView};
use spookery_protocol::{NoteView, PlaceView};

/// What one viewer is shown. Returned from the engine to the host table, which renders it as
/// a player or GM view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View {
    pub viewer_id: ActorId,
    /// Where the viewer is, with the exits they see there.
    pub place: PlaceView,
    pub place_prose: String,
    pub visible_actors: Vec<ActorId>,
    pub actor_names: Vec<String>,
    pub notes: Vec<NoteView>,
    /// The items lying in the viewer's place.
    pub items: Vec<ItemStack>,
    pub items_carried: Vec<ItemStack>,
    /// Which of `items_carried` the viewer wields.
    pub wielded_item_id: Option<String>,
    /// The viewer's own meters. A hidden meter is not here.
    pub meters: Vec<MeterView>,
    /// The world's meters. A hidden one is not here.
    pub world_meters: Vec<MeterView>,
    pub habits: Vec<String>,
    /// The places `place.exits` go to that the viewer's `map` knows, each with the exits the
    /// viewer saw there.
    pub nearby: Vec<PlaceView>,
    /// What keeps the viewer busy and until when, or empty when they are free.
    pub busy: String,
}

pub(crate) fn build_view_by_id<Lock>(
    session: &session::Session<Lock>,
    presence: &Presence,
    viewer_id: &str,
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
    if !world.actors().contains(&id) {
        return Err(Reason::UnknownId);
    }
    build_view(canon, world, presence, &id)
}

pub(crate) fn build_view(
    canon: &Canon,
    world: &ecs::World,
    presence: &Presence,
    viewer_id: &ActorId,
) -> Result<View, Reason> {
    let position = world.actors().get::<component::Place>(viewer_id).get();
    let position = position.ok_or(Reason::UnknownId)?;
    let (position_id, place_id) = (position.to_string(), position.place_id());
    let mut visible_actors: Vec<ActorId> = world.actors_at(&position_id);
    visible_actors.sort();

    let mut notes = Vec::new();
    for (note_id, note) in world.globals().get::<Note>(&globals::Id).iter() {
        if !note.is_read_by(viewer_id, place_id.map(PlaceId::as_str)) {
            continue;
        }
        notes.push(NoteView {
            id: note_id.to_string(),
            to_ids: note.to_ids.iter().map(ActorId::to_string).collect(),
            place_id: note.place_id.as_ref().map(PlaceId::to_string),
            text: note.text.clone(),
        });
    }

    let place_store = world.places();
    let actor_store = world.actors();
    let items = match place_id {
        Some(id) => build_item_stacks(canon, place_store.get::<component::Item>(id).iter())?,
        None => Vec::new(),
    };
    let items_carried =
        build_item_stacks(canon, actor_store.get::<component::Item>(viewer_id).iter())?;
    let wielded_item_id =
        (actor_store.get::<component::WieldedItem>(viewer_id).get()).map(str::to_string);

    let meters = shown_meters(canon, world, &entity::Id::Actor(viewer_id.clone()));
    let world_meters = shown_meters(canon, world, &entity::Id::Global);

    let habits = if actor_store.get::<component::ControlType>(viewer_id).is_pc() {
        Vec::new()
    } else {
        list_whose_if_holds(canon, world, presence, viewer_id)
    };
    let exits = crate::rules::travel::visible_exits(canon, world, viewer_id);
    let is_past_an_exit = |place_id: &str| exits.iter().any(|exit| exit.place_id == place_id);
    let nearby = crate::rules::lines::map(canon, world, viewer_id, is_past_an_exit)?;
    let actor_names = visible_actors
        .iter()
        .map(|id| canon.name(id.as_str()).map(str::to_string))
        .collect::<Option<_>>()
        .ok_or(Reason::UnknownId)?;
    let place_name = position.find_name(canon).ok_or(Reason::UnknownId)?;
    let place_prose = crate::rules::lines::place_prose(canon, world, viewer_id);
    let busy = crate::rules::lines::busy_line(canon, world, viewer_id).unwrap_or_default();

    Ok(View {
        viewer_id: viewer_id.clone(),
        place: PlaceView {
            place_id: position_id,
            name: place_name,
            exits,
        },
        place_prose,
        visible_actors,
        actor_names,
        notes,
        items,
        items_carried,
        wielded_item_id,
        meters,
        world_meters,
        habits,
        nearby,
        busy,
    })
}

/// Each item of `item_counts`, with its count and name.
fn build_item_stacks<'a>(
    canon: &Canon,
    item_counts: impl Iterator<Item = (&'a str, &'a u32)>,
) -> Result<Vec<ItemStack>, Reason> {
    item_counts
        .map(|(id, count)| {
            Ok(ItemStack {
                id: id.to_string(),
                count: *count,
                name: canon.name(id).ok_or(Reason::UnknownId)?.to_string(),
            })
        })
        .collect()
}

/// The meters `entity_id` carries that are not hidden, in id order.
fn shown_meters(canon: &Canon, world: &ecs::World, entity_id: &entity::Id) -> Vec<MeterView> {
    let defs = if *entity_id == entity::Id::Global {
        &canon.world_meters
    } else {
        &canon.meters
    };
    defs.keys()
        .filter_map(|id| {
            let def = canon
                .meter_def(entity_id.as_ref(), id)
                .filter(|def| !def.hidden)?;
            let value = world.entities().get::<Meter>(entity_id).value(id).ok()?;
            Some(MeterView {
                id: id.to_string(),
                name: def.name.clone(),
                value,
                max: def.max,
            })
        })
        .collect()
}
