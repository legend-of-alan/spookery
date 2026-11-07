//! The live save: each kind of entity's components, the open doors, and notes.

pub(crate) mod item;
pub(crate) mod key;
pub(crate) mod ledger;
pub(crate) mod meter;
pub(crate) mod modifier;
pub(crate) mod tracked;
pub(crate) mod write;

use std::borrow::Borrow;
use std::collections::BTreeMap;
use std::convert::Infallible;
use std::fmt;

use serde::{Deserialize, Serialize};

use item::{Items, Shift};
use key::{StateKey, StateValue};
use ledger::Ledger;
use meter::{MeterLive, Meters, Offset};
use modifier::{ModifierLive, Modifiers};
use tracked::{swap, Edit, EntityMaps, EntityStore, Entry, KeyStore, Parts, PartsMut, Store};

use crate::canon::Canon;
use crate::intent::{Target, Viewer};
use crate::rules::effect::EntityId;

/// The live save: each kind of entity's components, the notes, and the ledger of the round
/// writing it. `save.yaml` is this, field for field: the actors' records and stores, then the
/// places', the stories' and the world's stores, each by entity id. A place or a story is the
/// pack's, so the save lists none: what it holds is in its kind's stores.
///
/// Anyone may read any of it, through its getters. Only the writers in `write` change a record
/// or a component, and each keeps its undo in its store and goes on the ledger: no getter hands
/// out a way to change one, so round code can hold `&mut State` without a way around them. The
/// clock and revision numbers are plain fields: nothing tracks or rolls them back.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub schema: String,
    pub world_id: String,
    pub pack_rev: String,
    /// Engine rules version. Missing means 0, which is not the current version.
    #[serde(default)]
    pub rules_rev: u64,
    pub state_rev: u64,
    pub round_index: u64,
    /// Ticks since the world began.
    pub world_time: u64,
    pub last_round_at: String,
    actors: ActorComponents,
    #[serde(default)]
    places: PlaceComponents,
    #[serde(default)]
    stories: StoryComponents,
    #[serde(default)]
    world: WorldComponents,
    notes: Store<Vec<Note>, NoteEdit>,
    #[serde(skip)]
    ledger: Ledger,
}

/// Each flag set on each entity of a kind. A flag is only on or off, so it has no edits:
/// setting one puts it on, and clearing it takes it off.
pub type Flags = EntityStore<(), Infallible>;

/// Every actor's record and the components each holds, by actor id. An actor exists when it
/// has a record.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ActorComponents {
    records: KeyStore<ActorLive, ActorEdit>,
    items: Items,
    meters: Meters,
    modifiers: Modifiers,
    #[serde(with = "tracked::marks::by_entity")]
    flags: Flags,
}

/// The components each place holds, by place id, and the doors open out of it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PlaceComponents {
    items: Items,
    meters: Meters,
    #[serde(with = "tracked::marks::by_entity")]
    flags: Flags,
    /// The name of each exit whose door is open, by its place.
    #[serde(with = "tracked::marks::by_entity")]
    open_exits: EntityStore<(), Infallible>,
}

/// The meters and flags each story holds, by story id.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StoryComponents {
    meters: Meters,
    #[serde(with = "tracked::marks::by_entity")]
    flags: Flags,
}

/// The world's meters and flags. There is one world, so they are by key alone.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WorldComponents {
    meters: KeyStore<MeterLive, Offset>,
    #[serde(with = "tracked::marks")]
    flags: KeyStore<(), Infallible>,
}

impl ActorComponents {
    pub fn records(&self) -> &BTreeMap<String, ActorLive> {
        &self.records
    }

    pub fn items(&self) -> &EntityMaps<u32> {
        &self.items
    }

    pub fn meters(&self) -> &EntityMaps<MeterLive> {
        &self.meters
    }

    pub fn modifiers(&self) -> &EntityMaps<ModifierLive> {
        &self.modifiers
    }

    pub fn flags(&self) -> &EntityMaps<()> {
        &self.flags
    }

    /// Undo every change to the actors' stores whose seq is `seq` or later.
    fn undo_to(&mut self, seq: u64) {
        let ActorComponents {
            records,
            items,
            meters,
            modifiers,
            flags,
        } = self;
        records.undo_to(seq);
        items.undo_to(seq);
        meters.undo_to(seq);
        modifiers.undo_to(seq);
        flags.undo_to(seq);
    }

    fn clear_log(&mut self) {
        let ActorComponents {
            records,
            items,
            meters,
            modifiers,
            flags,
        } = self;
        records.clear_log();
        items.clear_log();
        meters.clear_log();
        modifiers.clear_log();
        flags.clear_log();
    }

    fn has_undo(&self) -> bool {
        let ActorComponents {
            records,
            items,
            meters,
            modifiers,
            flags,
        } = self;
        records.has_undo()
            || items.has_undo()
            || meters.has_undo()
            || modifiers.has_undo()
            || flags.has_undo()
    }

    /// Every component in the actors' stores, with the actor holding it: items, then meters,
    /// the modifiers that read them, then flags, each in actor and key order. Copying,
    /// removing and checking an actor walk this, so a component added here fails to compile
    /// until it is walked.
    pub(crate) fn parts(&self) -> Vec<(String, ActorPart)> {
        let ActorComponents {
            records: _,
            items,
            meters,
            modifiers,
            flags,
        } = self;
        let mut parts = Vec::new();
        for (id, key, count) in items.iter() {
            parts.push((id.clone(), ActorPart::Item(key.clone(), *count)));
        }
        for (id, key, live) in meters.iter() {
            parts.push((id.clone(), ActorPart::Meter(key.clone(), live.clone())));
        }
        for (id, key, live) in modifiers.iter() {
            parts.push((id.clone(), ActorPart::Modifier(key.clone(), live.clone())));
        }
        for (id, key, ()) in flags.iter() {
            parts.push((id.clone(), ActorPart::Flag(key.clone())));
        }
        parts
    }
}

/// One component an actor holds, by its key, with its value.
pub(crate) enum ActorPart {
    Item(String, u32),
    Meter(String, MeterLive),
    Modifier(String, ModifierLive),
    Flag(String),
}

impl PlaceComponents {
    pub fn items(&self) -> &EntityMaps<u32> {
        &self.items
    }

    pub fn meters(&self) -> &EntityMaps<MeterLive> {
        &self.meters
    }

    pub fn flags(&self) -> &EntityMaps<()> {
        &self.flags
    }

    pub fn open_exits(&self) -> &EntityMaps<()> {
        &self.open_exits
    }

    /// Undo every change to the places' stores whose seq is `seq` or later.
    fn undo_to(&mut self, seq: u64) {
        let PlaceComponents {
            items,
            meters,
            flags,
            open_exits,
        } = self;
        items.undo_to(seq);
        meters.undo_to(seq);
        flags.undo_to(seq);
        open_exits.undo_to(seq);
    }

    fn clear_log(&mut self) {
        let PlaceComponents {
            items,
            meters,
            flags,
            open_exits,
        } = self;
        items.clear_log();
        meters.clear_log();
        flags.clear_log();
        open_exits.clear_log();
    }

    fn has_undo(&self) -> bool {
        let PlaceComponents {
            items,
            meters,
            flags,
            open_exits,
        } = self;
        items.has_undo() || meters.has_undo() || flags.has_undo() || open_exits.has_undo()
    }
}

impl StoryComponents {
    pub fn meters(&self) -> &EntityMaps<MeterLive> {
        &self.meters
    }

    pub fn flags(&self) -> &EntityMaps<()> {
        &self.flags
    }

    /// Undo every change to the stories' stores whose seq is `seq` or later.
    fn undo_to(&mut self, seq: u64) {
        let StoryComponents { meters, flags } = self;
        meters.undo_to(seq);
        flags.undo_to(seq);
    }

    fn clear_log(&mut self) {
        let StoryComponents { meters, flags } = self;
        meters.clear_log();
        flags.clear_log();
    }

    fn has_undo(&self) -> bool {
        let StoryComponents { meters, flags } = self;
        meters.has_undo() || flags.has_undo()
    }
}

impl WorldComponents {
    pub fn meters(&self) -> &BTreeMap<String, MeterLive> {
        &self.meters
    }

    pub fn flags(&self) -> &BTreeMap<String, ()> {
        &self.flags
    }

    /// Undo every change to the world's stores whose seq is `seq` or later.
    fn undo_to(&mut self, seq: u64) {
        let WorldComponents { meters, flags } = self;
        meters.undo_to(seq);
        flags.undo_to(seq);
    }

    fn clear_log(&mut self) {
        let WorldComponents { meters, flags } = self;
        meters.clear_log();
        flags.clear_log();
    }

    fn has_undo(&self) -> bool {
        let WorldComponents { meters, flags } = self;
        meters.has_undo() || flags.has_undo()
    }
}

/// Declares `$name`, the id of one kind of entity: a string saved as itself, which an id of
/// another kind cannot stand in for.
macro_rules! string_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl Borrow<str> for $name {
            fn borrow(&self) -> &str {
                &self.0
            }
        }

        impl From<&str> for $name {
            fn from(id: &str) -> Self {
                $name(id.to_string())
            }
        }

        impl From<String> for $name {
            fn from(id: String) -> Self {
                $name(id)
            }
        }

        impl From<$name> for String {
            fn from(id: $name) -> Self {
                id.0
            }
        }
    };
}

string_id! {
    /// An actor's id, as the cast and the save's records key them.
    ActorId
}

string_id! {
    /// A place's id, as the pack's places key them.
    PlaceId
}

string_id! {
    /// A story's id, as the pack's stories key them.
    StoryId
}

/// One exit, by its place and its name: the address of its door. An exit holds nothing else.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ExitId {
    pub place: PlaceId,
    pub exit: String,
}

impl ExitId {
    /// The exit named `exit` out of `place`.
    pub fn new(place: &str, exit: &str) -> ExitId {
        ExitId {
            place: place.into(),
            exit: exit.to_string(),
        }
    }
}

/// The exit as a walker on it has it for their place: `exit:<place>.<exit>`.
impl fmt::Display for ExitId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "exit:{}.{}", self.place, self.exit)
    }
}

impl State {
    pub fn actors(&self) -> &ActorComponents {
        &self.actors
    }

    pub fn places(&self) -> &PlaceComponents {
        &self.places
    }

    pub fn stories(&self) -> &StoryComponents {
        &self.stories
    }

    pub fn world(&self) -> &WorldComponents {
        &self.world
    }

    pub fn notes(&self) -> &[Note] {
        &self.notes
    }

    /// The meters `entity` holds.
    pub fn meters_of(&self, entity: &EntityId) -> Parts<'_, MeterLive> {
        Parts(match entity {
            EntityId::Actor(id) => self.actors.meters.of(id.as_str()),
            EntityId::Place(id) => self.places.meters.of(id.as_str()),
            EntityId::Story(id) => self.stories.meters.of(id.as_str()),
            EntityId::World => Some(&*self.world.meters),
        })
    }

    /// The flags set on `entity`.
    pub fn flags_of(&self, entity: &EntityId) -> Parts<'_, ()> {
        Parts(match entity {
            EntityId::Actor(id) => self.actors.flags.of(id.as_str()),
            EntityId::Place(id) => self.places.flags.of(id.as_str()),
            EntityId::Story(id) => self.stories.flags.of(id.as_str()),
            EntityId::World => Some(&*self.world.flags),
        })
    }

    /// The items `entity` holds. `None` for a story or the world: only an actor or a place
    /// holds items.
    pub fn items_of(&self, entity: &EntityId) -> Option<Parts<'_, u32>> {
        Some(Parts(match entity {
            EntityId::Actor(id) => self.actors.items.of(id.as_str()),
            EntityId::Place(id) => self.places.items.of(id.as_str()),
            EntityId::Story(_) | EntityId::World => return None,
        }))
    }

    /// Meter `key` on `entity` to write, as `meters_of` finds it, with the key it is watched
    /// under and the ledger, borrowed apart.
    fn meters_of_mut<'a>(
        &'a mut self,
        entity: &'a EntityId,
        key: &str,
    ) -> (PartsMut<'a, MeterLive, Offset>, StateKey, &'a mut Ledger) {
        let State {
            actors,
            places,
            stories,
            world,
            ledger,
            ..
        } = self;
        let key = key.to_string();
        let (meters, watched) = match entity {
            EntityId::Actor(id) => (
                PartsMut::Stored(&mut actors.meters, id.as_str()),
                StateKey::ActorMeter(id.to_string(), key),
            ),
            EntityId::Place(id) => (
                PartsMut::Stored(&mut places.meters, id.as_str()),
                StateKey::PlaceMeter(id.to_string(), key),
            ),
            EntityId::Story(id) => (
                PartsMut::Stored(&mut stories.meters, id.as_str()),
                StateKey::StoryMeter(id.to_string(), key),
            ),
            EntityId::World => (
                PartsMut::World(&mut world.meters),
                StateKey::WorldMeter(key),
            ),
        };
        (meters, watched, ledger)
    }

    /// Flag `key` on `entity` to write, as `meters_of_mut` says.
    fn flags_of_mut<'a>(
        &'a mut self,
        entity: &'a EntityId,
        key: &str,
    ) -> (PartsMut<'a, (), Infallible>, StateKey, &'a mut Ledger) {
        let State {
            actors,
            places,
            stories,
            world,
            ledger,
            ..
        } = self;
        let key = key.to_string();
        let (flags, watched) = match entity {
            EntityId::Actor(id) => (
                PartsMut::Stored(&mut actors.flags, id.as_str()),
                StateKey::ActorFlag(id.to_string(), key),
            ),
            EntityId::Place(id) => (
                PartsMut::Stored(&mut places.flags, id.as_str()),
                StateKey::PlaceFlag(id.to_string(), key),
            ),
            EntityId::Story(id) => (
                PartsMut::Stored(&mut stories.flags, id.as_str()),
                StateKey::StoryFlag(id.to_string(), key),
            ),
            EntityId::World => (PartsMut::World(&mut world.flags), StateKey::WorldFlag(key)),
        };
        (flags, watched, ledger)
    }

    /// Item `id` on `entity` to write, as `meters_of_mut` says. `None` for a story or the
    /// world, as `items_of` says.
    fn items_of_mut<'a>(
        &'a mut self,
        entity: &'a EntityId,
        id: &str,
    ) -> Option<(PartsMut<'a, u32, Shift>, StateKey, &'a mut Ledger)> {
        let State {
            actors,
            places,
            ledger,
            ..
        } = self;
        let id = id.to_string();
        let (items, watched) = match entity {
            EntityId::Actor(actor) => (
                PartsMut::Stored(&mut actors.items, actor.as_str()),
                StateKey::ActorItem(actor.to_string(), id),
            ),
            EntityId::Place(place) => (
                PartsMut::Stored(&mut places.items, place.as_str()),
                StateKey::PlaceItem(place.to_string(), id),
            ),
            EntityId::Story(_) | EntityId::World => return None,
        };
        Some((items, watched, ledger))
    }

    /// Refuse a component on an actor the save holds no record of, as `UNKNOWN_ID`. A place
    /// and a story are the pack's, as the world is, so the save takes one on any of them.
    pub(crate) fn require_entity(&self, entity: &EntityId) -> Result<(), crate::Reason> {
        match entity {
            EntityId::Actor(id) if !self.actors.records.contains_key(id.as_str()) => {
                Err(crate::Reason::UnknownId)
            }
            _ => Ok(()),
        }
    }

    /// The value of meter `key` on `entity`: its base and offset, plus what an actor's
    /// modifiers add to it, inside the pack's bounds. `UNKNOWN_ID` when the pack or the entity
    /// has no such meter. Every read of a meter's value, and every change reported for one,
    /// goes through here.
    pub fn meter_value(
        &self,
        canon: &Canon,
        entity: &EntityId,
        key: &str,
    ) -> Result<i64, crate::Reason> {
        let def = (canon.meter_def(entity, key)).ok_or(crate::Reason::UnknownId)?;
        let live = (self.meters_of(entity).get(key)).ok_or(crate::Reason::UnknownId)?;
        let modifiers = match entity {
            EntityId::Actor(id) => modifier::sum(self.actors.modifiers.of(id.as_str()), key)?,
            EntityId::Place(_) | EntityId::Story(_) | EntityId::World => 0,
        };
        meter::current(def, live, modifiers)
    }

    /// How many of item `id` `entity` holds: an actor the save holds, or a place. `None` for
    /// any other entity.
    pub fn item_count(&self, entity: &EntityId, id: &str) -> Option<u32> {
        let items = self.items_of(entity)?;
        self.require_entity(entity).ok()?;
        Some(items.get(id).copied().unwrap_or(0))
    }

    /// Whether `entity` has flag `key` set.
    pub fn has_flag(&self, entity: &EntityId, key: &str) -> bool {
        self.flags_of(entity).get(key).is_some()
    }

    /// Whether the door of `exit` is open.
    pub fn is_exit_open(&self, exit: &ExitId) -> bool {
        (self.places.open_exits.get(exit.place.as_str(), &exit.exit)).is_some()
    }

    /// Undo every change to any store whose seq is `seq` or later.
    fn undo_to(&mut self, seq: u64) {
        self.actors.undo_to(seq);
        self.places.undo_to(seq);
        self.stories.undo_to(seq);
        self.world.undo_to(seq);
        self.notes.undo_to(seq);
    }

    /// Forget every store's undo.
    fn clear_log(&mut self) {
        self.actors.clear_log();
        self.places.clear_log();
        self.stories.clear_log();
        self.world.clear_log();
        self.notes.clear_log();
    }

    /// Whether any store can still undo a change.
    fn has_undo(&self) -> bool {
        self.actors.has_undo()
            || self.places.has_undo()
            || self.stories.has_undo()
            || self.world.has_undo()
            || self.notes.has_undo()
    }

    /// This save in the order its file holds: notes by id, and each note's readers and facts,
    /// and each actor's known facts, sorted. A sorted list is a different value only in order,
    /// so this writes past the stores' logs, and so only `accept` and `to_canonical` call it,
    /// when no write is left to roll back.
    fn normalize(&mut self) {
        let notes = self.notes.frontier_mut();
        notes.sort_by(|a, b| a.id.cmp(&b.id));
        for note in notes {
            sort_unique(&mut note.to);
            sort_unique(&mut note.fact_ids);
        }
        for actor in self.actors.records.frontier_mut().values_mut() {
            sort_unique(&mut actor.known_facts);
        }
    }

    /// A copy of this save in the order its file holds, as `normalize` says.
    pub(crate) fn to_canonical(&self) -> State {
        let mut canonical = self.clone();
        canonical.normalize();
        canonical
    }

    /// The seat actor `id` acts from.
    pub(crate) fn viewer_of(&self, id: &str) -> Option<Viewer> {
        self.actors.records.get(id).map(|actor| actor.viewer(id))
    }

    /// The actors in `place`, in id order.
    pub(crate) fn actors_at(&self, place: &str) -> Vec<String> {
        (self.actors.records.iter())
            .filter(|(_, actor)| actor.place == place)
            .map(|(id, _)| id.clone())
            .collect()
    }
}

/// Who is seated. The host's seats own it and hand it to each round; the save never holds it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Presence {
    pub players: u32,
    pub actors_present: Vec<String>,
}

/// One actor's record. Their items, meters, modifiers and flags are in the save's stores, under
/// them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActorLive {
    pub kind: ActorKind,
    pub place: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wielded: Option<String>,
    pub known_facts: Vec<String>,
    /// Each place this actor has stood at the end of a round, with the names of the exits
    /// they saw there then. Omitted when empty.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub visited: BTreeMap<String, Vec<String>>,
    pub act: Act,
}

/// Whether a live actor is a PC or an NPC.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorKind {
    Npc,
    Pc,
}

impl ActorLive {
    /// Busy means an action under way, or a cooldown whose `busy_until` is still after this
    /// tick. An action stays under way until the round that reaches its finish ends.
    pub fn is_busy(&self, world_time: u64) -> bool {
        self.act.pending.is_some() || self.act.busy_until.is_some_and(|until| until > world_time)
    }

    pub fn is_npc(&self) -> bool {
        self.kind == ActorKind::Npc
    }

    pub fn is_pc(&self) -> bool {
        self.kind == ActorKind::Pc
    }

    /// The seat this actor acts from, given its own id.
    pub fn viewer(&self, id: &str) -> Viewer {
        match self.kind {
            ActorKind::Pc => Viewer::Player(id.to_string()),
            ActorKind::Npc => Viewer::Npc(id.to_string()),
        }
    }

    /// What key `key` reads on this actor. `Empty` for a key on anything else.
    pub(crate) fn value(&self, key: &StateKey) -> StateValue {
        match key {
            StateKey::ActorPlace(_) => StateValue::Text(self.place.clone()),
            StateKey::ActorWielded(_) => {
                (self.wielded.clone()).map_or(StateValue::Empty, StateValue::Text)
            }
            StateKey::ActorKnownFact(_) => StateValue::List(self.known_facts.clone()),
            StateKey::ActorBusy(_) => self.act.busy_until.map_or(StateValue::Empty, number),
            StateKey::LastActRound(_) => number(self.act.last_act_round),
            StateKey::IdleActsThisWindow(_) => count(self.act.idle_acts_this_window),
            StateKey::LiveActsThisWindow(_) => count(self.act.live_acts_this_window),
            _ => StateValue::Empty,
        }
    }
}

/// One change to an actor's live record. Making it gives back its undo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActorEdit {
    /// The place they stand in, or the exit they walk.
    Place(String),
    Wielded(Option<String>),
    /// Facts they come to know, each put in sorted order. Its undo is `Forget`.
    Learn(Vec<String>),
    Forget(Vec<String>),
    /// The tick they are busy until, and the action under way.
    Busy(Option<u64>, Option<PendingAction>),
    LastActRound(u64),
    Acts(ActCounter, u32),
    /// The exits they saw at a place.
    Visited(Entry<Vec<String>>),
}

impl ActorEdit {
    /// The key this edit moves on actor `id`, or `None` for a change no key answers for.
    pub(crate) fn key(&self, id: &str) -> Option<StateKey> {
        let id = id.to_string();
        Some(match self {
            ActorEdit::Place(_) => StateKey::ActorPlace(id),
            ActorEdit::Wielded(_) => StateKey::ActorWielded(id),
            ActorEdit::Learn(_) | ActorEdit::Forget(_) => StateKey::ActorKnownFact(id),
            ActorEdit::Busy(..) => StateKey::ActorBusy(id),
            ActorEdit::LastActRound(_) => StateKey::LastActRound(id),
            ActorEdit::Acts(counter, _) => counter.key(id),
            ActorEdit::Visited(_) => return None,
        })
    }
}

impl Edit<ActorLive> for ActorEdit {
    fn apply(self, actor: &mut ActorLive) -> Option<Self> {
        Some(match self {
            ActorEdit::Place(place) => ActorEdit::Place(swap(&mut actor.place, place)?),
            ActorEdit::Wielded(item) => ActorEdit::Wielded(swap(&mut actor.wielded, item)?),
            ActorEdit::Learn(facts) => {
                let known = &mut actor.known_facts;
                let mut learned = Vec::new();
                for fact in facts {
                    if let Err(at) = known.binary_search(&fact) {
                        known.insert(at, fact.clone());
                        learned.push(fact);
                    }
                }
                (!learned.is_empty()).then_some(ActorEdit::Forget(learned))?
            }
            ActorEdit::Forget(facts) => {
                actor.known_facts.retain(|fact| !facts.contains(fact));
                ActorEdit::Learn(facts)
            }
            ActorEdit::Busy(until, pending) => {
                let act = &mut actor.act;
                if act.busy_until == until && act.pending == pending {
                    return None;
                }
                let until = std::mem::replace(&mut act.busy_until, until);
                ActorEdit::Busy(until, std::mem::replace(&mut act.pending, pending))
            }
            ActorEdit::LastActRound(round) => {
                ActorEdit::LastActRound(swap(&mut actor.act.last_act_round, round)?)
            }
            ActorEdit::Acts(counter, count) => {
                ActorEdit::Acts(counter, swap(counter.count_mut(&mut actor.act), count)?)
            }
            ActorEdit::Visited(edit) => ActorEdit::Visited(edit.apply(&mut actor.visited)?),
        })
    }
}

/// A meter's value as a watched value, `Empty` when it does not read.
pub(crate) fn reading(value: Result<i64, crate::Reason>) -> StateValue {
    value.map_or(StateValue::Empty, StateValue::Number)
}

fn count(value: u32) -> StateValue {
    StateValue::Number(i64::from(value))
}

pub(crate) fn number(value: u64) -> StateValue {
    StateValue::Number(i64::try_from(value).unwrap_or(i64::MAX))
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// How many times an actor has acted, and the action they are still on.
pub struct Act {
    pub last_act_round: u64,
    pub idle_acts_this_window: u32,
    pub live_acts_this_window: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub busy_until: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<PendingAction>,
}

/// One of the `Act` counts the end of its act window sets back to 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActCounter {
    IdleActsThisWindow,
    LiveActsThisWindow,
}

impl ActCounter {
    /// The key this count is tracked under for actor `id`.
    pub(crate) fn key(self, id: String) -> StateKey {
        match self {
            ActCounter::IdleActsThisWindow => StateKey::IdleActsThisWindow(id),
            ActCounter::LiveActsThisWindow => StateKey::LiveActsThisWindow(id),
        }
    }

    /// This count on `act`, to write.
    pub(crate) fn count_mut(self, act: &mut Act) -> &mut u32 {
        match self {
            ActCounter::IdleActsThisWindow => &mut act.idle_acts_this_window,
            ActCounter::LiveActsThisWindow => &mut act.live_acts_this_window,
        }
    }
}

/// A timed action that finishes when `busy_until` is reached: its verb, who queued it, the
/// target it was queued against, and the rule it started with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingAction {
    pub verb: String,
    pub source: crate::intent::Source,
    #[serde(default, skip_serializing_if = "Target::is_empty")]
    pub target: Target,
    pub rule: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Note {
    pub id: String,
    pub to: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<String>,
    pub fact_ids: Vec<String>,
    pub text: String,
}

/// A note left, or the newest note taken back, which is how leaving one is undone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NoteEdit {
    Leave(Note),
    TakeBack,
}

impl Edit<Vec<Note>> for NoteEdit {
    fn apply(self, notes: &mut Vec<Note>) -> Option<Self> {
        match self {
            NoteEdit::Leave(note) => {
                notes.push(note);
                Some(NoteEdit::TakeBack)
            }
            NoteEdit::TakeBack => notes.pop().map(NoteEdit::Leave),
        }
    }
}

/// The place and exit names in an exit's id, `exit:<place>.<exit>`: where a walker stands
/// until they arrive. `None` for any other id.
pub(crate) fn exit_parts(id: &str) -> Option<(&str, &str)> {
    let (place, exit) = id.strip_prefix("exit:")?.split_once('.')?;
    (!place.is_empty() && !exit.is_empty()).then_some((place, exit))
}

pub(crate) fn sort_unique(items: &mut Vec<String>) {
    items.sort();
    items.dedup();
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryLine {
    pub round_index: u64,
    pub actor_id: String,
    pub intent_id: String,
    pub fact_ids: Vec<String>,
    pub tags: Vec<String>,
    pub text: Option<String>,
}
