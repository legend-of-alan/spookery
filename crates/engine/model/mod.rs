//! The live save: places, actors, stories, the component stores, and notes.

pub(crate) mod item;
pub(crate) mod key;
pub(crate) mod ledger;
pub(crate) mod meter;
pub(crate) mod modifier;
pub(crate) mod tracked;
pub(crate) mod write;

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use item::Items;
use key::{StateKey, StateValue};
use ledger::{Ledger, Removed};
use meter::Meters;
use modifier::Modifiers;
use tracked::{swap, Edit, Entry, ReadOnly, Rewind, Store, Tracked};

use crate::canon::Canon;
use crate::intent::{Target, Viewer};
use crate::rules::effect::Holder;

/// The live save: each record, each component store, and the ledger of the round writing it.
/// `save.yaml` is this, field for field: the records by id, then each store by holder.
///
/// Anyone may read any of it. Only the writers in `write` change a record or a component, and
/// each goes on the ledger: no record table, store or record hands out a way to change it, so
/// round code can hold `&mut State` without a way around the ledger. The clock and revision
/// numbers are plain fields: nothing tracks or rolls them back.
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
    /// The places the save lists: each place someone can stand in. Its items, meters and
    /// flags are in the stores.
    pub places: ReadOnly<BTreeSet<String>>,
    pub actors: ReadOnly<BTreeMap<String, Tracked<ActorLive, ActorEdit>>>,
    /// The stories the save lists. A story's meters and flags are in the stores.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub stories: ReadOnly<BTreeSet<String>>,
    /// Every item count each actor and place holds.
    #[serde(default, skip_serializing_if = "Store::is_empty")]
    pub items: Items,
    /// Every live meter: each actor's, place's and story's, and the world's.
    #[serde(default, skip_serializing_if = "Store::is_empty")]
    pub meters: Meters,
    /// The modifiers each actor holds.
    #[serde(default, skip_serializing_if = "Store::is_empty")]
    pub modifiers: Modifiers,
    /// Every flag set, on any holder. An exit's door is open when the exit has its flag `open`.
    #[serde(
        default,
        skip_serializing_if = "Store::is_empty",
        with = "tracked::marks"
    )]
    pub flags: Flags,
    pub notes: Tracked<Vec<Note>, NoteEdit>,
    #[serde(skip)]
    ledger: Ledger,
}

/// Each flag set on each holder. A flag is only on or off, so it has no edits: setting one puts
/// it on, and clearing it takes it off.
pub type Flags = Store<(), ()>;

/// One tracked component of the save: a live record, the story a write listed, a meter, a
/// modifier or a flag on its holder, or the notes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ComponentId {
    Actor(String),
    Place(String),
    Story(String),
    Meter(Holder, String),
    Modifier(Holder, String),
    Flag(Holder, String),
    Item(Holder, String),
    Notes,
}

impl ComponentId {
    /// The store this component is in, as the file names it, and the holder it is on. A record
    /// and the notes are in no store.
    pub(crate) fn stored(&self) -> Option<(&'static str, &Holder)> {
        match self {
            ComponentId::Item(holder, _) => Some(("items", holder)),
            ComponentId::Meter(holder, _) => Some(("meters", holder)),
            ComponentId::Modifier(holder, _) => Some(("modifiers", holder)),
            ComponentId::Flag(holder, _) => Some(("flags", holder)),
            ComponentId::Actor(_)
            | ComponentId::Place(_)
            | ComponentId::Story(_)
            | ComponentId::Notes => None,
        }
    }
}

impl State {
    /// Every component in the stores, by id: items, meters, the modifiers that read those meters,
    /// then flags, each by holder. The one walk over every store: the load check reads it, and
    /// so do copying and removing an actor.
    pub(crate) fn components(&self) -> impl Iterator<Item = ComponentId> + '_ {
        let items =
            (self.items.iter()).map(|(at, id, _)| ComponentId::Item(at.clone(), id.clone()));
        let meters =
            (self.meters.iter()).map(|(at, key, _)| ComponentId::Meter(at.clone(), key.clone()));
        let modifiers = (self.modifiers.iter())
            .map(|(at, id, _)| ComponentId::Modifier(at.clone(), id.clone()));
        let flags =
            (self.flags.iter()).map(|(at, key, _)| ComponentId::Flag(at.clone(), key.clone()));
        items.chain(meters).chain(modifiers).chain(flags)
    }

    /// Whether the save lists `holder`: an actor, place or story with a record, an exit of a
    /// listed place, or the world.
    pub(crate) fn lists(&self, holder: &Holder) -> bool {
        match holder {
            Holder::Actor(id) => self.actors.contains_key(id),
            Holder::Place(id) | Holder::Exit(id, _) => self.places.contains(id),
            Holder::Story(id) => self.stories.contains(id),
            Holder::World => true,
        }
    }

    /// The value of meter `key` on `holder`: its base and offset, plus what the holder's
    /// modifiers add to it, inside the pack's bounds. `UNKNOWN_ID` when the pack or the holder
    /// has no such meter. Every read of a meter's value, and every change reported for one,
    /// goes through here.
    pub fn meter_value(
        &self,
        canon: &Canon,
        holder: &Holder,
        key: &str,
    ) -> Result<i64, crate::Reason> {
        let def = (canon.meter_def(holder, key)).ok_or(crate::Reason::UnknownId)?;
        let live = (self.meters.get(holder, key)).ok_or(crate::Reason::UnknownId)?;
        meter::current(def, live, modifier::sum(&self.modifiers, holder, key)?)
    }

    /// How many of item `id` `holder` holds: an actor or place the save lists. `None` for
    /// any other holder.
    pub fn item_count(&self, holder: &Holder, id: &str) -> Option<u32> {
        (item::holds_items(holder) && self.lists(holder))
            .then(|| self.items.get(holder, id).copied().unwrap_or(0))
    }

    /// Whether `holder` has flag `key` set.
    pub fn has_flag(&self, holder: &Holder, key: &str) -> bool {
        self.flags.get(holder, key).is_some()
    }

    /// The component `at` names, to roll back or rebase, when it has edits to.
    fn component_mut(&mut self, at: &ComponentId) -> Option<&mut dyn Rewind> {
        Some(match at {
            ComponentId::Actor(id) => self.actors.get_mut().get_mut(id)?,
            ComponentId::Meter(holder, key) => self.meters.tracked_mut(holder, key)?,
            ComponentId::Modifier(holder, id) => self.modifiers.tracked_mut(holder, id)?,
            ComponentId::Item(holder, id) => self.items.tracked_mut(holder, id)?,
            ComponentId::Notes => &mut self.notes,
            ComponentId::Place(_) | ComponentId::Story(_) | ComponentId::Flag(..) => return None,
        })
    }

    /// Take off the component `at` names, which a write put on.
    fn take_off(&mut self, at: &ComponentId) {
        match at {
            ComponentId::Actor(id) => drop(self.actors.get_mut().remove(id)),
            ComponentId::Place(id) => drop(self.places.get_mut().remove(id)),
            ComponentId::Story(id) => drop(self.stories.get_mut().remove(id)),
            ComponentId::Meter(holder, key) => drop(self.meters.take(holder, key)),
            ComponentId::Modifier(holder, id) => drop(self.modifiers.take(holder, id)),
            ComponentId::Flag(holder, key) => drop(self.flags.take(holder, key)),
            ComponentId::Item(holder, id) => drop(self.items.take(holder, id)),
            ComponentId::Notes => unreachable!("no write puts the notes on"),
        }
    }

    /// Put back a component a write took off, with the undo it held.
    fn put_back(&mut self, removed: Removed) {
        match removed {
            #[cfg(any(test, feature = "testing"))]
            Removed::Actor(id, record) => drop(self.actors.get_mut().insert(id, *record)),
            Removed::Meter(holder, key, part) => self.meters.put(holder, key, part),
            Removed::Modifier(holder, id, part) => self.modifiers.put(holder, id, part),
            Removed::Flag(holder, key) => self.flags.put(holder, key, Tracked::default()),
            Removed::Item(holder, id, part) => self.items.put(holder, id, part),
        }
    }

    /// This save in the order its file holds: notes by id, and each note's readers and facts,
    /// and each actor's known facts, sorted. A sorted list is a different value only in order,
    /// so this writes past the ledger, and so only `accept` and `to_canonical` call it, when
    /// no write is left to roll back.
    fn normalize(&mut self) {
        let notes = self.notes.frontier_mut();
        notes.sort_by(|a, b| a.id.cmp(&b.id));
        for note in notes {
            sort_unique(&mut note.to);
            sort_unique(&mut note.fact_ids);
        }
        for actor in self.actors.get_mut().values_mut() {
            sort_unique(&mut actor.frontier_mut().known_facts);
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
        self.actors.get(id).map(|actor| actor.viewer(id))
    }

    /// The actors in `place`, in id order.
    pub(crate) fn actors_at(&self, place: &str) -> Vec<String> {
        self.actors
            .iter()
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
