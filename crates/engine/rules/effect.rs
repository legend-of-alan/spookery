//! One effect: its kind, and the fields that kind carries.

use serde::de::Error as _;
use serde::ser::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

use crate::canon::Canon;
use crate::ecs::component::meter;
use crate::ecs::component::modifier::{self, Modifier};
use crate::ecs::component::Note;
use crate::ecs::id::{ActorId, ExitId, GlobalId, PlaceId};
use crate::ecs::{self, component, MemoryLine, Presence};
use crate::entity;
use crate::intent::{Intent, Viewer};
use crate::rules::visibility::can_see_fact;
use crate::Reason;

/// One effect. JSON is `kind`, `as`, `intent_id`, then the fields of `kind`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Effect {
    pub kind: EffectKind,
    /// Who the effect was written as, or `None` for a change rule on a world change.
    pub viewer: Option<Viewer>,
    pub intent_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EffectKind {
    // repeats module: the tag of an Effect, named as a pair with it
    Meter {
        entity: entity::Id,
        key: String,
        #[serde(flatten)]
        write: meter::Write,
    },
    Item {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        actor_id: Option<ActorId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        place: Option<PlaceId>,
        id: String,
        delta: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<i64>,
    },
    /// One application of modifier `id`, of magnitude `add`, to an actor, lasting `ticks`.
    Modifier {
        actor_id: ActorId,
        id: String,
        add: i64,
        ticks: u64,
    },
    Wield {
        actor_id: ActorId,
        id: String,
    },
    Unwield {
        actor_id: ActorId,
    },
    Location {
        actor_id: ActorId,
        from: String,
        to: String,
        /// Exact tick of a timed arrival. Instant moves leave this empty.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        arrived_at: Option<u64>,
    },
    Memory {
        actor_id: ActorId,
        fact_ids: Vec<String>,
        tags: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
    },
    Flag {
        entity: entity::Id,
        key: String,
        value: bool,
    },
    /// Open or shut the door of an exit. JSON writes the exit as `place` and `exit`.
    ExitOpen {
        #[serde(flatten)]
        exit: ExitId,
        open: bool,
    },
    Note {
        note_id: String,
        to_ids: Vec<ActorId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        place_id: Option<PlaceId>,
        fact_ids: Vec<String>,
        text: String,
    },
    Narrate {
        to: Vec<ActorId>,
        fact_ids: Vec<String>,
        text: String,
    },
    Reveal {
        actor_id: ActorId,
        fact: String,
    },
}

/// Writes `kind`, then `as` when there is one, then `intent_id`, then the fields of `kind`.
impl Serialize for Effect {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let Value::Object(mut fields) =
            serde_json::to_value(&self.kind).map_err(S::Error::custom)?
        else {
            return Err(S::Error::custom("effect kind is not an object"));
        };
        fields.shift_insert(1, "intent_id".into(), self.intent_id.clone().into());
        if let Some(viewer) = &self.viewer {
            fields.shift_insert(1, "as".into(), viewer.to_string().into());
        }
        fields.serialize(serializer)
    }
}

/// Reads what `serialize` writes. A `kind` the engine does not run is `UNKNOWN_EFFECT`.
impl<'de> Deserialize<'de> for Effect {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut fields = Map::deserialize(deserializer)?;
        let viewer = fields
            .shift_remove("as")
            .map(Viewer::deserialize)
            .transpose()
            .map_err(D::Error::custom)?;
        let intent_id = fields
            .shift_remove("intent_id")
            .ok_or_else(|| D::Error::missing_field("intent_id"))?;
        let intent_id = String::deserialize(intent_id).map_err(D::Error::custom)?;
        let variant = fields
            .get("kind")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let kind = EffectKind::deserialize(Value::Object(fields)).map_err(|err| {
            let unknown = variant.is_some_and(|variant| {
                err.to_string()
                    .starts_with(&format!("unknown variant `{variant}`"))
            });
            if unknown {
                D::Error::custom(Reason::UnknownEffect)
            } else {
                D::Error::custom(err)
            }
        })?;
        if let EffectKind::Item {
            actor_id, place, ..
        } = &kind
        {
            if actor_id.is_some() == place.is_some() {
                return Err(D::Error::custom(Reason::Schema));
            }
        }
        Ok(Effect {
            kind,
            viewer,
            intent_id,
        })
    }
}

impl Effect {
    pub(crate) fn new(viewer: Option<&Viewer>, intent_id: &str, kind: EffectKind) -> Self {
        Effect {
            kind,
            viewer: viewer.cloned(),
            intent_id: intent_id.to_string(),
        }
    }

    /// Write this effect into the round's working copy of the save, with `presence` seated.
    pub(crate) fn apply(
        self,
        canon: &Canon,
        world: &mut ecs::World,
        presence: &Presence,
        intent: &Intent,
    ) -> Result<(), Reason> {
        if !viewer_matches(intent, &self) {
            return Err(Reason::ViewerMismatch);
        }
        match self.kind {
            EffectKind::Meter { entity, key, write } => {
                let meter::Write::Offset(delta) = write;
                (world.entities_mut().get_mut::<component::Meter>(&entity)).shift(&key, delta)
            }
            EffectKind::Item {
                actor_id,
                place,
                id,
                delta,
                max,
            } => {
                match (actor_id, place) {
                    (Some(actor), None) => (world.actors_mut().get_mut::<component::Item>(&actor))
                        .shift(&id, delta, max),
                    (None, Some(place)) => (world.places_mut().get_mut::<component::Item>(&place))
                        .shift(&id, delta, max),
                    _ => Err(Reason::Schema),
                }
            }
            EffectKind::Modifier {
                actor_id,
                id,
                add,
                ticks,
            } => apply_modifier(canon, world, &actor_id, &id, add, ticks),
            EffectKind::Wield { actor_id, id } => {
                let item = canon.items.get(&id).ok_or(Reason::UnknownId)?;
                if !item.tags.iter().any(|tag| tag == "weapon") {
                    return Err(Reason::NotAWeapon);
                }
                let mut actors = world.actors_mut();
                (actors.get_mut::<component::WieldedItem>(&actor_id)).set(Some(&id))
            }
            EffectKind::Unwield { actor_id } => {
                let mut actors = world.actors_mut();
                (actors.get_mut::<component::WieldedItem>(&actor_id)).set(None)
            }
            EffectKind::Location {
                actor_id, from, to, ..
            } => move_actor(canon, world, presence, intent, &actor_id, &from, &to),
            EffectKind::Memory {
                actor_id,
                fact_ids,
                tags,
                text,
            } => remember(canon, world, intent, &actor_id, fact_ids, tags, text),
            EffectKind::Flag { entity, key, value } => {
                if !canon.carries_flag(entity.as_ref(), &key) {
                    return Err(Reason::UnknownId);
                }
                (world.entities_mut().get_mut::<component::Flag>(&entity)).set(&key, value)
            }
            EffectKind::ExitOpen { exit, open } => {
                canon
                    .has_door(&exit)
                    .then_some(())
                    .ok_or(Reason::UnknownId)?;
                world.places_mut().set_exit_open(&exit, open)
            }
            EffectKind::Note {
                note_id,
                to_ids,
                place_id,
                fact_ids,
                text,
            } => {
                if place_id
                    .as_ref()
                    .is_some_and(|place_id| !canon.places.contains_key(place_id))
                {
                    return Err(Reason::UnknownId);
                }
                let note = Note {
                    to_ids: to_ids.into_iter().collect(),
                    place_id,
                    fact_ids: fact_ids.into_iter().collect(),
                    text,
                };
                let mut globals = world.globals_mut();
                globals.get_mut::<Note>(&GlobalId).leave(&note_id, note)
            }
            EffectKind::Narrate { .. } => Ok(()),
            EffectKind::Reveal { actor_id, fact } => {
                if !canon.facts.contains_key(&fact) {
                    return Err(Reason::UnknownId);
                }
                let mut actors = world.actors_mut();
                (actors.get_mut::<component::KnownFacts>(&actor_id)).learn(&[fact])
            }
        }
    }
}

impl EffectKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            EffectKind::Meter { .. } => "meter",
            EffectKind::Item { .. } => "item",
            EffectKind::Modifier { .. } => "modifier",
            EffectKind::Wield { .. } => "wield",
            EffectKind::Unwield { .. } => "unwield",
            EffectKind::Location { .. } => "location",
            EffectKind::Memory { .. } => "memory",
            EffectKind::Flag { .. } => "flag",
            EffectKind::ExitOpen { .. } => "exit_open",
            EffectKind::Note { .. } => "note",
            EffectKind::Narrate { .. } => "narrate",
            EffectKind::Reveal { .. } => "reveal",
        }
    }
}

/// The effect's fields minus `kind`, keys in declaration order.
pub(crate) fn payload_json(effect: &Effect) -> String {
    let Ok(Value::Object(mut fields)) = serde_json::to_value(&effect.kind) else {
        unreachable!("effect kinds are objects")
    };
    fields.shift_remove("kind");
    serde_json::to_string(&fields).expect("effect payload")
}

fn viewer_matches(intent: &Intent, effect: &Effect) -> bool {
    effect.viewer == intent.actor
}

/// Apply one stack of modifier `id` to an actor that carries its meter, lasting `ticks`, as
/// `modifier::stack` says.
fn apply_modifier(
    canon: &Canon,
    world: &mut ecs::World,
    actor_id: &ActorId,
    id: &str,
    add: i64,
    ticks: u64,
) -> Result<(), Reason> {
    let desc = canon.modifiers.get(id).ok_or(Reason::UnknownId)?;
    let carried = canon.meter_desc(entity::IdRef::Actor(actor_id), &desc.meter);
    carried.ok_or(Reason::UnknownId)?;
    let now = world.world_time;
    let held = world.actors().get::<Modifier>(actor_id).get(id);
    let mut next = held.cloned().unwrap_or_else(|| Modifier {
        meter: desc.meter.clone(),
        until: now,
        max_ticks: 0,
        stacks: Vec::new(),
    });
    modifier::stack(desc, &mut next, add, ticks, now);
    (world.actors_mut().get_mut::<Modifier>(actor_id)).set(id, Some(next))
}

/// Move the intent's own actor from `from` to `to`, a place or the exit they walk. A PC moves
/// only while a player is seated.
fn move_actor(
    canon: &Canon,
    world: &mut ecs::World,
    presence: &Presence,
    intent: &Intent,
    actor_id: &ActorId,
    from: &str,
    to: &str,
) -> Result<(), Reason> {
    if intent.actor_id() != Some(actor_id) {
        return Err(Reason::ActorMismatch);
    }
    let actors = world.actors();
    if presence.players == 0 && actors.get::<component::ControlType>(actor_id).is_pc() {
        return Err(Reason::TableEmpty);
    }
    let place = actors.get::<component::Place>(actor_id).get();
    if place.ok_or(Reason::UnknownId)? != from {
        return Err(Reason::OutOfRange);
    }
    let on_exit = crate::rules::travel::exit_stood_on(canon, to).is_some();
    if !canon.places.contains_key(to) && !on_exit {
        return Err(Reason::NoExit);
    }
    (world.actors_mut().get_mut::<component::Place>(actor_id)).set(to)
}

fn remember(
    canon: &Canon,
    world: &mut ecs::World,
    intent: &Intent,
    actor_id: &ActorId,
    mut fact_ids: Vec<String>,
    mut tags: Vec<String>,
    text: Option<String>,
) -> Result<(), Reason> {
    let Some(viewer) = intent
        .actor
        .as_ref()
        .filter(|viewer| viewer.actor_id() == actor_id)
    else {
        return Err(Reason::ActorMismatch);
    };
    if text.as_ref().is_some_and(|text| text.chars().count() > 240) {
        return Err(Reason::Schema);
    }
    for fact in &fact_ids {
        if !can_see_fact(canon, world, viewer, fact) {
            return Err(Reason::IllegalKnowledge);
        }
    }
    world.count_memory_write()?;
    ecs::sort_unique(&mut fact_ids);
    let mut actors = world.actors_mut();
    (actors.get_mut::<component::KnownFacts>(actor_id)).learn(&fact_ids)?;
    ecs::sort_unique(&mut tags);
    world.push_memory(MemoryLine {
        round_index: world.round_index,
        actor_id: actor_id.clone(),
        intent_id: intent.id.clone(),
        fact_ids,
        tags,
        text,
    });
    Ok(())
}
