//! The live save: places, actors, stories, world meters, and notes.

pub(crate) mod key;
pub(crate) mod meter;
pub(crate) mod modifier;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use key::StateKey;
use meter::MeterLive;
use modifier::ModifierLive;

use crate::canon::OPEN;
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
    pub places: BTreeMap<String, PlaceLive>,
    pub actors: BTreeMap<String, ActorLive>,
    /// The meters and flags each story holds. Omitted when empty.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub stories: BTreeMap<String, StoryLive>,
    /// The meters `world/meters.yaml` declares, held once by the world. Omitted when empty.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub world_meters: BTreeMap<String, MeterLive>,
    pub notes: Vec<Note>,
    /// World flags. Omitted from the save when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flags: Vec<String>,
}

impl State {
    /// The current value of meter `key` on `holder`, or `UNKNOWN_ID` when it does not carry it.
    pub fn meter_value(
        &self,
        canon: &crate::canon::Canon,
        holder: &Holder,
        key: &str,
    ) -> Result<i64, crate::Reason> {
        let def = canon
            .meter_def(holder, key)
            .ok_or(crate::Reason::UnknownId)?;
        let live = self
            .meters_of(holder)
            .and_then(|meters| meters.get(key))
            .ok_or(crate::Reason::UnknownId)?;
        meter::current(def, live, self.modifier_sum(holder, key)?)
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

    /// The live meters `holder` carries, to write.
    pub(crate) fn meters_mut(
        &mut self,
        holder: &Holder,
    ) -> Result<&mut BTreeMap<String, MeterLive>, crate::Reason> {
        match holder {
            Holder::Actor(id) => Ok(&mut self.actor_mut(id)?.meters),
            Holder::Place(id) => Ok(&mut self
                .places
                .get_mut(id)
                .ok_or(crate::Reason::UnknownId)?
                .meters),
            Holder::Story(id) => Ok(&mut self
                .stories
                .get_mut(id)
                .ok_or(crate::Reason::UnknownId)?
                .meters),
            Holder::Exit(..) => Err(crate::Reason::UnknownId),
            Holder::World => Ok(&mut self.world_meters),
        }
    }

    /// Whether `holder` has flag `key` set.
    pub(crate) fn has_flag(&self, holder: &Holder, key: &str) -> bool {
        if let Some(flag) = Flag::resolve(holder, key) {
            return flag.get(self);
        }
        let flags = match holder {
            Holder::Story(id) => self.stories.get(id).map(|story| &story.flags),
            Holder::World => Some(&self.flags),
            Holder::Actor(_) | Holder::Place(_) | Holder::Exit(..) => None,
        };
        flags.is_some_and(|flags| flags.iter().any(|flag| flag == key))
    }

    /// Set or clear a story or world flag `key`, and say whether it changed. A story gets a
    /// live record on its first flag. The other holders' flags go through `Flag`.
    pub(crate) fn set_flag(
        &mut self,
        holder: &Holder,
        key: &str,
        value: bool,
    ) -> Result<bool, crate::Reason> {
        if self.has_flag(holder, key) == value {
            return Ok(false);
        }
        let flags = match holder {
            Holder::Story(id) => &mut self.stories.entry(id.clone()).or_default().flags,
            Holder::World => &mut self.flags,
            Holder::Actor(_) | Holder::Place(_) | Holder::Exit(..) => {
                return Err(crate::Reason::UnknownId)
            }
        };
        if value {
            flags.push(key.to_string());
        } else {
            flags.retain(|flag| flag != key);
        }
        Ok(true)
    }

    /// Actor `id`'s live record to write, or `UNKNOWN_ID`.
    pub(crate) fn actor_mut(&mut self, id: &str) -> Result<&mut ActorLive, crate::Reason> {
        self.actors.get_mut(id).ok_or(crate::Reason::UnknownId)
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

/// One flag resolved to where the save keeps it: the list it sits in, under its owner, and
/// the entry that marks it set. An actor's flag is its key in the actor's `flags`; an exit's
/// `open` is the exit's name in its place's `open`. The handle holds no borrow into `State`.
#[derive(Clone, Debug)]
pub(crate) struct Flag {
    /// The flag as the budget, change rules and the GM name it.
    pub(crate) key: StateKey,
    owner: String,
    entry: String,
    list: ListFn,
    list_mut: ListMutFn,
}

impl Flag {
    /// Where flag `key` on `holder` lives, or `None` when the holder's flags are not kept
    /// this way. The one place that knows the layout.
    pub(crate) fn resolve(holder: &Holder, key: &str) -> Option<Flag> {
        let (owner, entry, list, list_mut): (&str, &str, ListFn, ListMutFn) = match holder {
            Holder::Actor(id) => (
                id,
                key,
                |state, id| state.actors.get(id).map(|actor| &actor.flags),
                |state, id| Ok(&mut state.actor_mut(id)?.flags),
            ),
            Holder::Place(id) => (
                id,
                key,
                |state, id| Some(state.places.get(id).map_or(&NO_FLAGS, |place| &place.flags)),
                |state, id| Ok(&mut state.places.entry(id.to_string()).or_default().flags),
            ),
            Holder::Exit(place, exit) if key == OPEN => (
                place,
                exit,
                |state, id| Some(state.places.get(id).map_or(&NO_FLAGS, |place| &place.open)),
                |state, id| Ok(&mut state.places.entry(id.to_string()).or_default().open),
            ),
            Holder::Exit(..) | Holder::Story(_) | Holder::World => return None,
        };
        Some(Flag {
            key: StateKey::Flag(holder.clone(), key.to_string()),
            owner: owner.to_string(),
            entry: entry.to_string(),
            list,
            list_mut,
        })
    }

    /// Whether this flag is set.
    pub(crate) fn get(&self, state: &State) -> bool {
        self.value(state) == Some(true)
    }

    /// Whether this flag is set, or `None` when its actor is absent. A place with no live
    /// record yet still carries its flags, all clear.
    pub(crate) fn value(&self, state: &State) -> Option<bool> {
        (self.list)(state, &self.owner).map(|list| list.contains(&self.entry))
    }

    /// Set or clear this flag, and say whether it changed. A place gets a live record on its
    /// first entry.
    pub(crate) fn set(&self, state: &mut State, value: bool) -> Result<bool, crate::Reason> {
        if self.get(state) == value {
            return Ok(false);
        }
        let list = (self.list_mut)(state, &self.owner)?;
        if value {
            list.push(self.entry.clone());
        } else {
            list.retain(|entry| entry != &self.entry);
        }
        Ok(true)
    }
}

/// The flags of a place that has no live record yet.
static NO_FLAGS: Vec<String> = Vec::new();

/// Reads and writes one kind of flag list, given its owner's id.
type ListFn = for<'s> fn(&'s State, &str) -> Option<&'s Vec<String>>;
type ListMutFn = for<'s> fn(&'s mut State, &str) -> Result<&'s mut Vec<String>, crate::Reason>;

/// One live part of the save a write keeps whole: an actor, a place, a story, the world's
/// meters and flags, or the notes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Live {
    Actor(String),
    Place(String),
    Story(String),
    World,
    Notes,
}

/// A live part as it was, `None` for a record that did not exist yet.
pub(crate) enum Kept {
    Actor(String, Option<Box<ActorLive>>),
    Place(String, Option<PlaceLive>),
    Story(String, Option<StoryLive>),
    World(BTreeMap<String, MeterLive>, Vec<String>),
    Notes(Vec<Note>),
}

impl State {
    /// A copy of `live` as it is now.
    pub(crate) fn keep(&self, live: &Live) -> Kept {
        match live {
            Live::Actor(id) => Kept::Actor(id.clone(), self.actors.get(id).cloned().map(Box::new)),
            Live::Place(id) => Kept::Place(id.clone(), self.places.get(id).cloned()),
            Live::Story(id) => Kept::Story(id.clone(), self.stories.get(id).cloned()),
            Live::World => Kept::World(self.world_meters.clone(), self.flags.clone()),
            Live::Notes => Kept::Notes(self.notes.clone()),
        }
    }

    /// Put a kept part back as it was.
    pub(crate) fn restore(&mut self, kept: Kept) {
        fn put<T>(map: &mut BTreeMap<String, T>, id: String, live: Option<T>) {
            match live {
                Some(live) => map.insert(id, live),
                None => map.remove(&id),
            };
        }
        match kept {
            Kept::Actor(id, live) => put(&mut self.actors, id, live.map(|live| *live)),
            Kept::Place(id, live) => put(&mut self.places, id, live),
            Kept::Story(id, live) => put(&mut self.stories, id, live),
            Kept::World(meters, flags) => (self.world_meters, self.flags) = (meters, flags),
            Kept::Notes(notes) => self.notes = notes,
        }
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

/// The meters a story file declares and the flags set on that story.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoryLive {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub meters: BTreeMap<String, MeterLive>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flags: Vec<String>,
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

    /// Drop the wielded id once the actor no longer holds it.
    pub(crate) fn release_wielded(&mut self) -> bool {
        let Some(id) = self.wielded.clone() else {
            return false;
        };
        if self.item_count(&id) > 0 {
            return false;
        }
        self.wielded = None;
        true
    }
}

/// Add `delta` to a count. Removing from a count of 0 is `MISSING_ITEM`.
/// A shortfall from a positive count is `OUT_OF_RANGE`. A positive `max` caps the result.
pub(crate) fn shift_count(
    counts: &mut BTreeMap<String, u32>,
    id: &str,
    delta: i64,
    max: Option<i64>,
) -> Result<bool, crate::Reason> {
    let current = i64::from(counts.get(id).copied().unwrap_or(0));
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
    let stored = u32::try_from(next).map_err(|_| crate::Reason::OutOfRange)?;
    let changed = stored != u32::try_from(current).unwrap_or(0);
    if stored == 0 {
        counts.remove(id);
    } else {
        counts.insert(id.to_string(), stored);
    }
    Ok(changed)
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
#[derive(Clone, Copy)]
pub(crate) enum ActCounter {
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
