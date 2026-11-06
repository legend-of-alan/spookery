//! The live save: places, actors, stories, world meters, and notes.

pub(crate) mod key;
pub(crate) mod meter;
pub(crate) mod modifier;
pub(crate) mod tracked;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use key::{StateKey, StateValue};
use meter::{MeterLive, Offset};
use modifier::ModifierLive;
use tracked::{swap, Count, Edit, Entry, Member, Rewind, Tracked};

use crate::canon::{Canon, OPEN};
use crate::intent::{Target, Viewer};
use crate::rules::effect::Holder;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
    pub places: BTreeMap<String, Tracked<PlaceLive, PlaceEdit>>,
    pub actors: BTreeMap<String, Tracked<ActorLive, ActorEdit>>,
    /// The meters and flags each story holds. Omitted when empty.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub stories: BTreeMap<String, Tracked<StoryLive, StoryEdit>>,
    /// The meters `world/meters.yaml` declares, held once by the world. Omitted when empty.
    #[serde(default, skip_serializing_if = "tracked::is_empty_map")]
    pub world_meters: Tracked<BTreeMap<String, MeterLive>, Offset>,
    pub notes: Tracked<Vec<Note>, NoteEdit>,
    /// World flags. Omitted from the save when empty.
    #[serde(default, skip_serializing_if = "tracked::is_empty")]
    pub flags: Tracked<Vec<String>, Member>,
}

/// One tracked component of the save: a live record, or one of the world's lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ComponentId {
    Actor(String),
    Place(String),
    Story(String),
    WorldMeters,
    WorldFlags,
    Notes,
}

impl State {
    /// The current value of meter `key` on `holder`, or `UNKNOWN_ID` when it does not carry it.
    pub fn meter_value(
        &self,
        canon: &Canon,
        holder: &Holder,
        key: &str,
    ) -> Result<i64, crate::Reason> {
        let meters = self.meters_of(holder).ok_or(crate::Reason::UnknownId)?;
        let added = self.modifier_sum(holder, key)?;
        meter_value(canon, holder, meters, key, added)
    }

    /// What `holder`'s modifiers add to its meter `key`. Only an actor holds modifiers.
    pub(crate) fn modifier_sum(&self, holder: &Holder, key: &str) -> Result<i64, crate::Reason> {
        match holder {
            Holder::Actor(id) => self
                .actors
                .get(id)
                .map_or(Ok(0), |actor| modifier::sum(&actor.modifiers, key)),
            Holder::World | Holder::Place(_) | Holder::Story(_) | Holder::Exit(..) => Ok(0),
        }
    }

    /// The live meters `holder` carries.
    pub(crate) fn meters_of(&self, holder: &Holder) -> Option<&BTreeMap<String, MeterLive>> {
        match holder {
            Holder::Actor(id) => self.actors.get(id).map(|actor| &actor.meters),
            Holder::Place(id) => self.places.get(id).map(|place| &place.meters),
            Holder::Story(id) => self.stories.get(id).map(|story| &story.meters),
            Holder::Exit(..) => None,
            Holder::World => Some(&self.world_meters),
        }
    }

    /// Whether `holder` has flag `key` set. An exit's `open` is its name in its place's list.
    pub(crate) fn has_flag(&self, holder: &Holder, key: &str) -> bool {
        let (flags, entry) = match holder {
            Holder::Actor(id) => (self.actors.get(id).map(|actor| &actor.flags), key),
            Holder::Place(id) => (self.places.get(id).map(|place| &place.flags), key),
            Holder::Story(id) => (self.stories.get(id).map(|story| &story.flags), key),
            Holder::Exit(place, exit) if key == OPEN => (
                self.places.get(place).map(|place| &place.open),
                exit.as_str(),
            ),
            Holder::Exit(..) => (None, key),
            Holder::World => (Some(&*self.flags), key),
        };
        flags.is_some_and(|flags| flags.iter().any(|flag| flag == entry))
    }

    /// The tracked part `at` names, to roll back or rebase, when it is there.
    pub(crate) fn record_mut(&mut self, at: &ComponentId) -> Option<&mut dyn Rewind> {
        Some(match at {
            ComponentId::Actor(id) => self.actors.get_mut(id)?,
            ComponentId::Place(id) => self.places.get_mut(id)?,
            ComponentId::Story(id) => self.stories.get_mut(id)?,
            ComponentId::WorldMeters => &mut self.world_meters,
            ComponentId::WorldFlags => &mut self.flags,
            ComponentId::Notes => &mut self.notes,
        })
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

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaceLive {
    pub items: BTreeMap<String, u32>,
    /// The meters the place file declares. Omitted when empty.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub meters: BTreeMap<String, MeterLive>,
    /// The flags set on this place. Omitted when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flags: Vec<String>,
    /// The names of this place's exits whose door is open. Omitted when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub open: Vec<String>,
}

impl PlaceLive {
    /// What key `key` reads on place `id`: an item count, a meter's value, a flag, or whether a
    /// door is open. `Empty` for any other key.
    pub(crate) fn value(&self, canon: &Canon, id: &str, key: &StateKey) -> StateValue {
        match key {
            StateKey::PlaceItem(_, item) => count(self.items.get(item).copied().unwrap_or(0)),
            StateKey::PlaceMeter(_, key) => {
                let holder = Holder::Place(id.to_string());
                reading(meter_value(canon, &holder, &self.meters, key, 0))
            }
            StateKey::PlaceFlag(_, key) => StateValue::Bool(self.flags.contains(key)),
            StateKey::ExitOpen(_, exit) => StateValue::Bool(self.open.contains(exit)),
            _ => StateValue::Empty,
        }
    }
}

/// One change to a place's live record. Making it gives back its undo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlaceEdit {
    Item(Count),
    Meter(Offset),
    Flag(Member),
    /// An exit's door, by the exit's name.
    Open(Member),
}

impl PlaceEdit {
    /// The key this edit moves on place `id`.
    pub(crate) fn key(&self, id: &str) -> StateKey {
        let id = id.to_string();
        match self {
            PlaceEdit::Item(edit) => StateKey::PlaceItem(id, edit.id.clone()),
            PlaceEdit::Meter(edit) => StateKey::PlaceMeter(id, edit.key.clone()),
            PlaceEdit::Flag(edit) => StateKey::PlaceFlag(id, edit.id().to_string()),
            PlaceEdit::Open(edit) => StateKey::ExitOpen(id, edit.id().to_string()),
        }
    }
}

impl Edit<PlaceLive> for PlaceEdit {
    fn apply(self, place: &mut PlaceLive) -> Option<Self> {
        Some(match self {
            PlaceEdit::Item(edit) => PlaceEdit::Item(edit.apply(&mut place.items)?),
            PlaceEdit::Meter(edit) => PlaceEdit::Meter(edit.apply(&mut place.meters)?),
            PlaceEdit::Flag(edit) => PlaceEdit::Flag(edit.apply(&mut place.flags)?),
            PlaceEdit::Open(edit) => PlaceEdit::Open(edit.apply(&mut place.open)?),
        })
    }
}

/// The meters a story file declares and the flags set on that story.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoryLive {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub meters: BTreeMap<String, MeterLive>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flags: Vec<String>,
}

impl StoryLive {
    /// What key `key` reads on story `id`: a meter's value or a flag. `Empty` for any other key.
    pub(crate) fn value(&self, canon: &Canon, id: &str, key: &StateKey) -> StateValue {
        match key {
            StateKey::StoryMeter(_, key) => {
                let holder = Holder::Story(id.to_string());
                reading(meter_value(canon, &holder, &self.meters, key, 0))
            }
            StateKey::StoryFlag(_, key) => StateValue::Bool(self.flags.contains(key)),
            _ => StateValue::Empty,
        }
    }
}

/// One change to a story's live record. Making it gives back its undo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoryEdit {
    Meter(Offset),
    Flag(Member),
}

impl StoryEdit {
    /// The key this edit moves on story `id`.
    pub(crate) fn key(&self, id: &str) -> StateKey {
        let id = id.to_string();
        match self {
            StoryEdit::Meter(edit) => StateKey::StoryMeter(id, edit.key.clone()),
            StoryEdit::Flag(edit) => StateKey::StoryFlag(id, edit.id().to_string()),
        }
    }
}

impl Edit<StoryLive> for StoryEdit {
    fn apply(self, story: &mut StoryLive) -> Option<Self> {
        Some(match self {
            StoryEdit::Meter(edit) => StoryEdit::Meter(edit.apply(&mut story.meters)?),
            StoryEdit::Flag(edit) => StoryEdit::Flag(edit.apply(&mut story.flags)?),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActorLive {
    pub kind: ActorKind,
    pub place: String,
    pub flags: Vec<String>,
    pub meters: BTreeMap<String, MeterLive>,
    /// The modifiers this actor holds, by id. Omitted when empty.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub modifiers: BTreeMap<String, ModifierLive>,
    pub inventory: BTreeMap<String, u32>,
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

    pub fn item_count(&self, id: &str) -> u32 {
        self.inventory.get(id).copied().unwrap_or(0)
    }

    /// The value of meter `key` on actor `id`, with what this actor's modifiers add to it.
    pub(crate) fn meter_value(
        &self,
        canon: &Canon,
        id: &str,
        key: &str,
    ) -> Result<i64, crate::Reason> {
        let added = modifier::sum(&self.modifiers, key)?;
        let holder = Holder::Actor(id.to_string());
        meter_value(canon, &holder, &self.meters, key, added)
    }

    /// What key `key` reads on actor `id`. `Empty` for a key on anything else.
    pub(crate) fn value(&self, canon: &Canon, id: &str, key: &StateKey) -> StateValue {
        match key {
            StateKey::ActorPlace(_) => StateValue::Text(self.place.clone()),
            StateKey::ActorFlag(_, key) => StateValue::Bool(self.flags.contains(key)),
            StateKey::ActorMeter(_, key) => reading(self.meter_value(canon, id, key)),
            StateKey::ActorModifier(_, key) => {
                (self.modifiers.get(key)).map_or(StateValue::Empty, |live| {
                    StateValue::Text(format!(
                        "{:?} until {} max_ticks {}",
                        live.stacks, live.until, live.max_ticks
                    ))
                })
            }
            StateKey::ActorItem(_, item) => count(self.item_count(item)),
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
    Flag(Member),
    Meter(Offset),
    Modifier(Entry<ModifierLive>),
    Item(Count),
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
            ActorEdit::Flag(edit) => StateKey::ActorFlag(id, edit.id().to_string()),
            ActorEdit::Meter(edit) => StateKey::ActorMeter(id, edit.key.clone()),
            ActorEdit::Modifier(edit) => StateKey::ActorModifier(id, edit.id.clone()),
            ActorEdit::Item(edit) => StateKey::ActorItem(id, edit.id.clone()),
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
            ActorEdit::Flag(edit) => ActorEdit::Flag(edit.apply(&mut actor.flags)?),
            ActorEdit::Meter(edit) => ActorEdit::Meter(edit.apply(&mut actor.meters)?),
            ActorEdit::Modifier(edit) => ActorEdit::Modifier(edit.apply(&mut actor.modifiers)?),
            ActorEdit::Item(edit) => ActorEdit::Item(edit.apply(&mut actor.inventory)?),
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

/// The value of meter `key` in `meters`, which `holder` carries, plus `added` from modifiers,
/// or `UNKNOWN_ID` when the pack or the holder has no such meter.
pub(crate) fn meter_value(
    canon: &Canon,
    holder: &Holder,
    meters: &BTreeMap<String, MeterLive>,
    key: &str,
    added: i64,
) -> Result<i64, crate::Reason> {
    let def = (canon.meter_def(holder, key)).ok_or(crate::Reason::UnknownId)?;
    let live = meters.get(key).ok_or(crate::Reason::UnknownId)?;
    meter::current(def, live, added)
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

/// `current` plus `delta`. Removing from a count of 0 is `MISSING_ITEM`. A shortfall from a
/// positive count is `OUT_OF_RANGE`. A positive `max` caps the result.
pub(crate) fn shift_count(
    current: u32,
    delta: i64,
    max: Option<i64>,
) -> Result<u32, crate::Reason> {
    let current = i64::from(current);
    let mut next = current
        .checked_add(delta)
        .ok_or(crate::Reason::OutOfRange)?;
    if next < 0 {
        if current == 0 {
            return Err(crate::Reason::MissingItem);
        }
        return Err(crate::Reason::OutOfRange);
    }
    if delta > 0 {
        if let Some(cap) = max {
            if cap < 0 {
                return Err(crate::Reason::OutOfRange);
            }
            if next > cap {
                next = cap;
            }
        }
    }
    u32::try_from(next).map_err(|_| crate::Reason::OutOfRange)
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
