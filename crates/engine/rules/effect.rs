//! One effect: its kind, and the fields that kind carries.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::canon::Canon;
use crate::intent::{Intent, Viewer};
use crate::model::item;
use crate::model::meter::{self, MeterWrite};
use crate::model::modifier::{self, ModifierLive};
use crate::model::{self, ActorEdit, ExitId, MemoryLine, Note, State};
use crate::rules::visibility::can_see_fact;
use crate::Reason;

/// One effect. JSON is `kind`, `as`, `intent_id`, then the fields of `kind`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "EffectRecord", try_from = "EffectRecord")]
pub struct Effect {
    pub kind: EffectKind,
    /// Who the effect was written as, or `None` for a change rule on a world change.
    pub viewer: Option<Viewer>,
    pub intent_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EffectKind {
    Meter {
        entity: EntityId,
        key: String,
        write: MeterWrite,
    },
    Item {
        actor_id: Option<String>,
        place: Option<String>,
        id: String,
        delta: i64,
        max: Option<i64>,
    },
    /// One application of modifier `id`, of magnitude `add`, to an actor, lasting `ticks`.
    Modifier {
        actor_id: String,
        id: String,
        add: i64,
        ticks: u64,
    },
    Wield {
        actor_id: String,
        id: String,
    },
    Unwield {
        actor_id: String,
    },
    Location {
        actor_id: String,
        from: String,
        to: String,
        /// Exact tick of a timed arrival. Instant moves leave this empty.
        arrived_at: Option<u64>,
    },
    Memory {
        actor_id: String,
        fact_ids: Vec<String>,
        tags: Vec<String>,
        text: Option<String>,
    },
    Flag {
        entity: EntityId,
        key: String,
        value: bool,
    },
    /// The door of `exit` opens, or shuts when `open` is false.
    ExitOpen {
        exit: ExitId,
        open: bool,
    },
    Note {
        note_id: String,
        to: Vec<String>,
        place: Option<String>,
        fact_ids: Vec<String>,
        text: String,
    },
    Narrate {
        to: Vec<String>,
        fact_ids: Vec<String>,
        text: String,
    },
    Reveal {
        actor_id: String,
        fact: String,
    },
}

impl Effect {
    pub(crate) fn new(viewer: Option<&Viewer>, intent_id: &str, kind: EffectKind) -> Self {
        Effect {
            kind,
            viewer: viewer.cloned(),
            intent_id: intent_id.to_string(),
        }
    }

    /// Write this effect into the save, under the rules of `canon`.
    pub(crate) fn apply(
        self,
        canon: &Canon,
        state: &mut State,
        intent: &Intent,
    ) -> Result<(), Reason> {
        if !viewer_matches(intent, &self) {
            return Err(Reason::ViewerMismatch);
        }
        match self.kind {
            EffectKind::Meter { entity, key, write } => {
                write_meter(canon, state, &entity, &key, write)
            }
            EffectKind::Item {
                actor_id,
                place,
                id,
                delta,
                max,
            } => {
                let entity = match (actor_id, place) {
                    (Some(actor_id), None) => EntityId::Actor(actor_id),
                    (None, Some(place)) => EntityId::Place(place),
                    _ => return Err(Reason::Schema),
                };
                shift_item(canon, state, &entity, &id, delta, max)
            }
            EffectKind::Modifier {
                actor_id,
                id,
                add,
                ticks,
            } => apply_modifier(canon, state, &actor_id, &id, add, ticks),
            EffectKind::Wield { actor_id, id } => wield(canon, state, &actor_id, &id),
            EffectKind::Unwield { actor_id } => unwield(state, &actor_id),
            EffectKind::Location {
                actor_id, from, to, ..
            } => move_actor(canon, state, intent, &actor_id, &from, &to),
            EffectKind::Memory {
                actor_id,
                fact_ids,
                tags,
                text,
            } => remember(canon, state, intent, &actor_id, fact_ids, tags, text),
            EffectKind::Flag { entity, key, value } => set_flag(canon, state, &entity, &key, value),
            EffectKind::ExitOpen { exit, open } => set_exit_open(canon, state, &exit, open),
            EffectKind::Note {
                note_id,
                to,
                place,
                fact_ids,
                text,
            } => leave_note(canon, state, note_id, to, place, fact_ids, text),
            EffectKind::Narrate { .. } => Ok(()),
            EffectKind::Reveal { actor_id, fact } => reveal(canon, state, &actor_id, &fact),
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

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum EffectRecord {
    Meter {
        #[serde(rename = "as", default, skip_serializing_if = "Option::is_none")]
        as_viewer: Option<Viewer>,
        intent_id: String,
        entity: EntityId,
        key: String,
        #[serde(flatten)]
        write: MeterWrite,
    },
    Item {
        #[serde(rename = "as", default, skip_serializing_if = "Option::is_none")]
        as_viewer: Option<Viewer>,
        intent_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        actor_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        place: Option<String>,
        id: String,
        delta: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<i64>,
    },
    Modifier {
        #[serde(rename = "as", default, skip_serializing_if = "Option::is_none")]
        as_viewer: Option<Viewer>,
        intent_id: String,
        actor_id: String,
        id: String,
        add: i64,
        ticks: u64,
    },
    Wield {
        #[serde(rename = "as", default, skip_serializing_if = "Option::is_none")]
        as_viewer: Option<Viewer>,
        intent_id: String,
        actor_id: String,
        id: String,
    },
    Unwield {
        #[serde(rename = "as", default, skip_serializing_if = "Option::is_none")]
        as_viewer: Option<Viewer>,
        intent_id: String,
        actor_id: String,
    },
    Location {
        #[serde(rename = "as", default, skip_serializing_if = "Option::is_none")]
        as_viewer: Option<Viewer>,
        intent_id: String,
        actor_id: String,
        from: String,
        to: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        arrived_at: Option<u64>,
    },
    Memory {
        #[serde(rename = "as", default, skip_serializing_if = "Option::is_none")]
        as_viewer: Option<Viewer>,
        intent_id: String,
        actor_id: String,
        fact_ids: Vec<String>,
        tags: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
    },
    Flag {
        #[serde(rename = "as", default, skip_serializing_if = "Option::is_none")]
        as_viewer: Option<Viewer>,
        intent_id: String,
        entity: EntityId,
        key: String,
        value: bool,
    },
    ExitOpen {
        #[serde(rename = "as", default, skip_serializing_if = "Option::is_none")]
        as_viewer: Option<Viewer>,
        intent_id: String,
        place: String,
        exit: String,
        open: bool,
    },
    Note {
        #[serde(rename = "as", default, skip_serializing_if = "Option::is_none")]
        as_viewer: Option<Viewer>,
        intent_id: String,
        note_id: String,
        to: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        place: Option<String>,
        fact_ids: Vec<String>,
        text: String,
    },
    Narrate {
        #[serde(rename = "as", default, skip_serializing_if = "Option::is_none")]
        as_viewer: Option<Viewer>,
        intent_id: String,
        to: Vec<String>,
        fact_ids: Vec<String>,
        text: String,
    },
    Reveal {
        #[serde(rename = "as", default, skip_serializing_if = "Option::is_none")]
        as_viewer: Option<Viewer>,
        intent_id: String,
        actor_id: String,
        fact: String,
    },
    #[serde(other)]
    Unknown,
}

impl From<Effect> for EffectRecord {
    fn from(effect: Effect) -> Self {
        let as_viewer = effect.viewer;
        let intent_id = effect.intent_id;
        match effect.kind {
            EffectKind::Meter { entity, key, write } => Self::Meter {
                as_viewer,
                intent_id,
                entity,
                key,
                write,
            },
            EffectKind::Item {
                actor_id,
                place,
                id,
                delta,
                max,
            } => Self::Item {
                as_viewer,
                intent_id,
                actor_id,
                place,
                id,
                delta,
                max,
            },
            EffectKind::Modifier {
                actor_id,
                id,
                add,
                ticks,
            } => Self::Modifier {
                as_viewer,
                intent_id,
                actor_id,
                id,
                add,
                ticks,
            },
            EffectKind::Wield { actor_id, id } => Self::Wield {
                as_viewer,
                intent_id,
                actor_id,
                id,
            },
            EffectKind::Unwield { actor_id } => Self::Unwield {
                as_viewer,
                intent_id,
                actor_id,
            },
            EffectKind::Location {
                actor_id,
                from,
                to,
                arrived_at,
            } => Self::Location {
                as_viewer,
                intent_id,
                actor_id,
                from,
                to,
                arrived_at,
            },
            EffectKind::Memory {
                actor_id,
                fact_ids,
                tags,
                text,
            } => Self::Memory {
                as_viewer,
                intent_id,
                actor_id,
                fact_ids,
                tags,
                text,
            },
            EffectKind::Flag { entity, key, value } => Self::Flag {
                as_viewer,
                intent_id,
                entity,
                key,
                value,
            },
            EffectKind::ExitOpen { exit, open } => Self::ExitOpen {
                as_viewer,
                intent_id,
                place: exit.place,
                exit: exit.exit,
                open,
            },
            EffectKind::Note {
                note_id,
                to,
                place,
                fact_ids,
                text,
            } => Self::Note {
                as_viewer,
                intent_id,
                note_id,
                to,
                place,
                fact_ids,
                text,
            },
            EffectKind::Narrate { to, fact_ids, text } => Self::Narrate {
                as_viewer,
                intent_id,
                to,
                fact_ids,
                text,
            },
            EffectKind::Reveal { actor_id, fact } => Self::Reveal {
                as_viewer,
                intent_id,
                actor_id,
                fact,
            },
        }
    }
}

impl TryFrom<EffectRecord> for Effect {
    type Error = Reason;

    fn try_from(record: EffectRecord) -> Result<Self, Self::Error> {
        let (kind, as_viewer, intent_id) = match record {
            EffectRecord::Meter {
                as_viewer,
                intent_id,
                entity,
                key,
                write,
            } => (
                EffectKind::Meter { entity, key, write },
                as_viewer,
                intent_id,
            ),
            EffectRecord::Item {
                as_viewer,
                intent_id,
                actor_id,
                place,
                id,
                delta,
                max,
            } => {
                if actor_id.is_some() == place.is_some() {
                    return Err(Reason::Schema);
                }
                (
                    EffectKind::Item {
                        actor_id,
                        place,
                        id,
                        delta,
                        max,
                    },
                    as_viewer,
                    intent_id,
                )
            }
            EffectRecord::Modifier {
                as_viewer,
                intent_id,
                actor_id,
                id,
                add,
                ticks,
            } => (
                EffectKind::Modifier {
                    actor_id,
                    id,
                    add,
                    ticks,
                },
                as_viewer,
                intent_id,
            ),
            EffectRecord::Wield {
                as_viewer,
                intent_id,
                actor_id,
                id,
            } => (EffectKind::Wield { actor_id, id }, as_viewer, intent_id),
            EffectRecord::Unwield {
                as_viewer,
                intent_id,
                actor_id,
            } => (EffectKind::Unwield { actor_id }, as_viewer, intent_id),
            EffectRecord::Location {
                as_viewer,
                intent_id,
                actor_id,
                from,
                to,
                arrived_at,
            } => (
                EffectKind::Location {
                    actor_id,
                    from,
                    to,
                    arrived_at,
                },
                as_viewer,
                intent_id,
            ),
            EffectRecord::Memory {
                as_viewer,
                intent_id,
                actor_id,
                fact_ids,
                tags,
                text,
            } => (
                EffectKind::Memory {
                    actor_id,
                    fact_ids,
                    tags,
                    text,
                },
                as_viewer,
                intent_id,
            ),
            EffectRecord::Flag {
                as_viewer,
                intent_id,
                entity,
                key,
                value,
            } => (
                EffectKind::Flag { entity, key, value },
                as_viewer,
                intent_id,
            ),
            EffectRecord::ExitOpen {
                as_viewer,
                intent_id,
                place,
                exit,
                open,
            } => {
                let exit = ExitId { place, exit };
                (EffectKind::ExitOpen { exit, open }, as_viewer, intent_id)
            }
            EffectRecord::Note {
                as_viewer,
                intent_id,
                note_id,
                to,
                place,
                fact_ids,
                text,
            } => (
                EffectKind::Note {
                    note_id,
                    to,
                    place,
                    fact_ids,
                    text,
                },
                as_viewer,
                intent_id,
            ),
            EffectRecord::Narrate {
                as_viewer,
                intent_id,
                to,
                fact_ids,
                text,
            } => (
                EffectKind::Narrate { to, fact_ids, text },
                as_viewer,
                intent_id,
            ),
            EffectRecord::Reveal {
                as_viewer,
                intent_id,
                actor_id,
                fact,
            } => (EffectKind::Reveal { actor_id, fact }, as_viewer, intent_id),
            EffectRecord::Unknown => return Err(Reason::UnknownEffect),
        };
        Ok(Effect {
            kind,
            viewer: as_viewer,
            intent_id,
        })
    }
}

/// The effect's record minus `kind`, `as` and `intent_id`, keys in declaration order.
pub(crate) fn payload_json(effect: &Effect) -> String {
    let record = serde_json::to_value(effect).expect("effect payload");
    let serde_json::Value::Object(mut fields) = record else {
        unreachable!("effect records are objects")
    };
    for key in ["kind", "as", "intent_id"] {
        fields.shift_remove(key);
    }
    serde_json::to_string(&fields).expect("effect payload")
}

fn viewer_matches(intent: &Intent, effect: &Effect) -> bool {
    effect.viewer == intent.actor
}

/// Who holds a meter, a flag or an item count: an actor, a place, a story, or the world. Saved
/// as `world`, `{actor: id}`, `{place: id}` or `{story: id}`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityId {
    Actor(String),
    Place(String),
    Story(String),
    World,
}

/// The entity as a rule's `who` names it: an actor id, `place:<id>`, `story:<id>` or `world`.
impl fmt::Display for EntityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EntityId::Actor(id) => write!(f, "{id}"),
            EntityId::Place(id) => write!(f, "place:{id}"),
            EntityId::Story(id) => write!(f, "story:{id}"),
            EntityId::World => write!(f, "world"),
        }
    }
}

/// Read an entity back from the words `Display` writes, as the save's stores key them.
impl std::str::FromStr for EntityId {
    type Err = String;

    fn from_str(word: &str) -> Result<Self, String> {
        let named = |id: &str| (!id.is_empty()).then(|| id.to_string());
        let entity = if let Some(id) = word.strip_prefix("place:") {
            named(id).map(EntityId::Place)
        } else if let Some(id) = word.strip_prefix("story:") {
            named(id).map(EntityId::Story)
        } else if word == "world" {
            Some(EntityId::World)
        } else {
            (!word.contains(':'))
                .then(|| named(word).map(EntityId::Actor))
                .flatten()
        };
        entity.ok_or_else(|| format!("'{word}' names no entity"))
    }
}

/// Write a meter `entity` carries, inside the bounds the pack gives it.
pub(crate) fn write_meter(
    canon: &Canon,
    state: &mut State,
    entity: &EntityId,
    key: &str,
    write: MeterWrite,
) -> Result<(), Reason> {
    let def = canon.meter_def(entity, key).ok_or(Reason::UnknownId)?;
    let held = state.meters.get(entity, key);
    let edit = meter::write(def, held.ok_or(Reason::UnknownId)?, write)?;
    state.edit_meter(canon, entity, key, edit)
}

/// Add `delta` to the count `entity` has of item `id`: an actor's, or a place's the pack
/// defines. An actor who no longer holds the item they wield lets go of it.
pub(crate) fn shift_item(
    canon: &Canon,
    state: &mut State,
    entity: &EntityId,
    id: &str,
    delta: i64,
    max: Option<i64>,
) -> Result<(), Reason> {
    if !item::holds_items(entity) {
        return Err(Reason::Schema);
    }
    if matches!(entity, EntityId::Place(place) if !canon.places.contains_key(place)) {
        return Err(Reason::UnknownId);
    }
    let held = state.item_count(entity, id).ok_or(Reason::UnknownId)?;
    let count = item::shift_count(held, delta, max)?;
    state.set_items(entity, id, count)?;
    let EntityId::Actor(actor) = entity else {
        return Ok(());
    };
    let wielded = state.actors[actor].wielded.as_ref();
    let lets_go = wielded.is_some_and(|item| state.item_count(entity, item) == Some(0));
    if lets_go {
        state.edit_actor(actor, ActorEdit::Wielded(None))?;
    }
    Ok(())
}

/// Apply one stack of modifier `id` to an actor that carries its meter, lasting `ticks`, as
/// `modifier::stack` says.
fn apply_modifier(
    canon: &Canon,
    state: &mut State,
    actor_id: &str,
    id: &str,
    add: i64,
    ticks: u64,
) -> Result<(), Reason> {
    let def = canon.modifiers.get(id).ok_or(Reason::UnknownId)?;
    let entity = EntityId::Actor(actor_id.to_string());
    canon
        .meter_def(&entity, &def.meter)
        .ok_or(Reason::UnknownId)?;
    let now = state.world_time;
    state.actors.get(actor_id).ok_or(Reason::UnknownId)?;
    let mut next = (state.modifiers.get(&entity, id))
        .cloned()
        .unwrap_or_else(|| ModifierLive {
            meter: def.meter.clone(),
            until: now,
            max_ticks: 0,
            stacks: Vec::new(),
        });
    modifier::stack(def, &mut next, add, ticks, now);
    state.put_modifier(canon, actor_id, id, Some(next))
}

fn wield(canon: &Canon, state: &mut State, actor_id: &str, item_id: &str) -> Result<(), Reason> {
    let item = canon.items.get(item_id).ok_or(Reason::UnknownId)?;
    if !item.tags.iter().any(|tag| tag == "weapon") {
        return Err(Reason::NotAWeapon);
    }
    let held = state.item_count(&EntityId::Actor(actor_id.to_string()), item_id);
    if held.ok_or(Reason::UnknownId)? == 0 {
        return Err(Reason::MissingItem);
    }
    state.edit_actor(actor_id, ActorEdit::Wielded(Some(item_id.to_string())))
}

fn unwield(state: &mut State, actor_id: &str) -> Result<(), Reason> {
    state.edit_actor(actor_id, ActorEdit::Wielded(None))
}

fn move_actor(
    canon: &Canon,
    state: &mut State,
    intent: &Intent,
    actor_id: &str,
    from: &str,
    to: &str,
) -> Result<(), Reason> {
    if intent.actor_id() != Some(actor_id) {
        return Err(Reason::ActorMismatch);
    }
    let in_canon = canon.places.contains_key(to);
    // A walker stands on the exit until they arrive. The exit is no place of its own.
    let on_exit = crate::rules::travel::exit_stood_on(canon, to).is_some();
    let actor = state.actors.get(actor_id).ok_or(Reason::UnknownId)?;
    if state.presence().players == 0 && actor.is_pc() {
        return Err(Reason::TableEmpty);
    }
    if actor.place != from {
        return Err(Reason::OutOfRange);
    }
    if !in_canon && !on_exit {
        return Err(Reason::NoExit);
    }
    state.edit_actor(actor_id, ActorEdit::Place(to.to_string()))
}

fn remember(
    canon: &Canon,
    state: &mut State,
    intent: &Intent,
    actor_id: &str,
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
        if !can_see_fact(canon, state, viewer, fact) {
            return Err(Reason::IllegalKnowledge);
        }
    }
    state.count_memory_write()?;
    model::sort_unique(&mut fact_ids);
    learn(state, actor_id, &fact_ids)?;
    model::sort_unique(&mut tags);
    state.push_memory(MemoryLine {
        round_index: state.round_index,
        actor_id: actor_id.to_string(),
        intent_id: intent.id.clone(),
        fact_ids,
        tags,
        text,
    });
    Ok(())
}

/// Add `facts` to what actor `actor_id` knows.
fn learn(state: &mut State, actor_id: &str, facts: &[String]) -> Result<(), Reason> {
    state.edit_actor(actor_id, ActorEdit::Learn(facts.to_vec()))
}

/// Set or clear a flag `entity` carries.
fn set_flag(
    canon: &Canon,
    state: &mut State,
    entity: &EntityId,
    key: &str,
    value: bool,
) -> Result<(), Reason> {
    if !canon.carries_flag(entity, key) {
        return Err(Reason::UnknownId);
    }
    state.set_flag(entity, key, value)
}

/// Open or shut the door of `exit`, an exit of the pack's with an `if`.
fn set_exit_open(
    canon: &Canon,
    state: &mut State,
    exit: &ExitId,
    open: bool,
) -> Result<(), Reason> {
    if !canon.has_door(exit) {
        return Err(Reason::UnknownId);
    }
    state.set_exit_open(exit, open)
}

fn reveal(canon: &Canon, state: &mut State, actor_id: &str, fact: &str) -> Result<(), Reason> {
    if !canon.facts.contains_key(fact) {
        return Err(Reason::UnknownId);
    }
    learn(state, actor_id, &[fact.to_string()])
}

fn leave_note(
    canon: &Canon,
    state: &mut State,
    note_id: String,
    mut to: Vec<String>,
    place: Option<String>,
    mut fact_ids: Vec<String>,
    text: String,
) -> Result<(), Reason> {
    if state.notes.iter().any(|note| note.id == note_id) {
        return Err(Reason::UnknownId);
    }
    if let Some(place_id) = &place {
        if !canon.places.contains_key(place_id) {
            return Err(Reason::UnknownId);
        }
    }
    model::sort_unique(&mut to);
    model::sort_unique(&mut fact_ids);
    state.leave_note(Note {
        id: note_id,
        to,
        place,
        fact_ids,
        text,
    })
}
