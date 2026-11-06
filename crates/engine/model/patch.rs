//! One change a round asks of its save: a stored field and the value it takes. Connects the
//! writers (effects, costs and the round's own writes) to `Draft::apply`, the one place a
//! round's save changes.

use std::collections::BTreeMap;
use std::mem;

use crate::canon::Canon;
use crate::model::key::{StateKey, StateValue};
use crate::model::meter::{self, MeterLive};
use crate::model::modifier::ModifierLive;
use crate::model::{ActCounter, Note, PendingAction, State};
use crate::rules::effect::Holder;
use crate::Reason;

/// One stored field of the save and the value it takes. Swapping a patch in gives back the
/// patch that holds the value it replaced, so the undo log is a list of patches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Patch {
    /// The place an actor stands in, or the exit they walk.
    ActorPlace { actor: String, place: String },
    /// Whether a holder has flag `key`. An exit's one flag is `open`.
    Flag {
        holder: Holder,
        key: String,
        on: bool,
    },
    /// A meter's stored base and offset. Its value adds the holder's modifiers.
    Meter {
        holder: Holder,
        key: String,
        live: MeterLive,
    },
    /// Modifier `id` on an actor, `None` once it is gone.
    Modifier {
        actor: String,
        id: String,
        held: Option<ModifierLive>,
    },
    /// How many of item `id` an actor or a place holds. A count of 0 is no entry.
    Item {
        holder: Holder,
        id: String,
        count: u32,
    },
    /// The item an actor wields.
    Wielded { actor: String, item: Option<String> },
    /// The facts an actor knows, sorted.
    KnownFacts { actor: String, facts: Vec<String> },
    /// The tick an actor is busy until, and the action under way.
    Busy {
        actor: String,
        until: Option<u64>,
        pending: Option<PendingAction>,
    },
    /// The last round an actor acted in.
    LastActRound { actor: String, round: u64 },
    /// One of an actor's act counts.
    Acts {
        actor: String,
        counter: ActCounter,
        count: u32,
    },
    /// The exits an actor saw at `place`, `None` before they stood there.
    Visited {
        actor: String,
        place: String,
        exits: Option<Vec<String>>,
    },
    /// Note `id`, `None` before it is left.
    Note { id: String, note: Option<Note> },
    /// Whether a place or a story has a live record.
    Record { holder: Holder, here: bool },
}

impl Patch {
    /// Swap this patch's value with the one `state` holds: afterwards the patch holds the old
    /// value, so swapping it again puts the save back. A record the field sits in that is not
    /// there is `UNKNOWN_ID`, and then neither side changes.
    pub(crate) fn swap(&mut self, state: &mut State) -> Result<(), Reason> {
        match self {
            Patch::ActorPlace { actor, place } => {
                mem::swap(&mut state.actor_mut(actor)?.place, place);
            }
            Patch::Flag { holder, key, on } => {
                let was = state.has_flag(holder, key);
                if was != *on {
                    let (flags, entry) = state.flags_mut(holder, key)?;
                    if *on {
                        flags.push(entry);
                    } else {
                        flags.retain(|flag| *flag != entry);
                    }
                }
                *on = was;
            }
            Patch::Meter { holder, key, live } => {
                let meters = state.meters_mut(holder)?;
                mem::swap(meters.get_mut(key).ok_or(Reason::UnknownId)?, live);
            }
            Patch::Modifier { actor, id, held } => {
                swap_entry(&mut state.actor_mut(actor)?.modifiers, id, held);
            }
            Patch::Item { holder, id, count } => {
                let counts = match holder {
                    Holder::Actor(actor) => &mut state.actor_mut(actor)?.inventory,
                    Holder::Place(place) => {
                        &mut state.places.get_mut(place).ok_or(Reason::UnknownId)?.items
                    }
                    Holder::Story(_) | Holder::Exit(..) | Holder::World => {
                        return Err(Reason::Schema)
                    }
                };
                let mut held = (*count > 0).then_some(*count);
                swap_entry(counts, id, &mut held);
                *count = held.unwrap_or(0);
            }
            Patch::Wielded { actor, item } => {
                mem::swap(&mut state.actor_mut(actor)?.wielded, item);
            }
            Patch::KnownFacts { actor, facts } => {
                mem::swap(&mut state.actor_mut(actor)?.known_facts, facts);
            }
            Patch::Busy {
                actor,
                until,
                pending,
            } => {
                let act = &mut state.actor_mut(actor)?.act;
                mem::swap(&mut act.busy_until, until);
                mem::swap(&mut act.pending, pending);
            }
            Patch::LastActRound { actor, round } => {
                mem::swap(&mut state.actor_mut(actor)?.act.last_act_round, round);
            }
            Patch::Acts {
                actor,
                counter,
                count,
            } => {
                mem::swap(counter.count_mut(&mut state.actor_mut(actor)?.act), count);
            }
            Patch::Visited {
                actor,
                place,
                exits,
            } => {
                swap_entry(&mut state.actor_mut(actor)?.visited, place, exits);
            }
            Patch::Note { id, note } => {
                let at = state.notes.iter().position(|left| left.id == *id);
                let old = at.map(|at| state.notes.remove(at));
                state.notes.extend(mem::replace(note, old));
            }
            Patch::Record { holder, here } => {
                *here = match holder {
                    Holder::Place(id) => set_present(&mut state.places, id, *here),
                    Holder::Story(id) => set_present(&mut state.stories, id, *here),
                    Holder::Actor(_) | Holder::Exit(..) | Holder::World => {
                        return Err(Reason::Schema)
                    }
                };
            }
        }
        Ok(())
    }

    /// The key this patch writes, or `None` for a write no key answers for.
    pub(crate) fn key(&self) -> Option<StateKey> {
        Some(match self {
            Patch::ActorPlace { actor, .. } => StateKey::ActorPlace(actor.clone()),
            Patch::Flag { holder, key, .. } => StateKey::flag(holder, key),
            Patch::Meter { holder, key, .. } => StateKey::meter(holder, key)?,
            Patch::Modifier { actor, id, .. } => StateKey::ActorModifier(actor.clone(), id.clone()),
            Patch::Item { holder, id, .. } => StateKey::item(holder, id)?,
            Patch::Wielded { actor, .. } => StateKey::ActorWielded(actor.clone()),
            Patch::KnownFacts { actor, .. } => StateKey::ActorKnownFact(actor.clone()),
            Patch::Busy { actor, .. } => StateKey::ActorBusy(actor.clone()),
            Patch::LastActRound { actor, .. } => StateKey::LastActRound(actor.clone()),
            Patch::Acts { actor, counter, .. } => counter.key(actor.clone()),
            Patch::Note { id, .. } => StateKey::Note(id.clone()),
            Patch::Visited { .. } | Patch::Record { .. } => return None,
        })
    }

    /// How this patch's value reads as its key's value. A meter reads through the pack, with
    /// the holder's modifiers as `state` has them.
    pub(crate) fn value(&self, canon: &Canon, state: &State) -> StateValue {
        match self {
            Patch::ActorPlace { place, .. } => StateValue::Text(place.clone()),
            Patch::Flag { on, .. } => StateValue::Bool(*on),
            Patch::Meter { holder, key, live } => canon
                .meter_def(holder, key)
                .zip(state.modifier_sum(holder, key).ok())
                .and_then(|(def, added)| meter::current(def, live, added).ok())
                .map_or(StateValue::Empty, StateValue::Number),
            Patch::Modifier { held, .. } => held.as_ref().map_or(StateValue::Empty, |live| {
                StateValue::Text(format!(
                    "{:?} until {} max_ticks {}",
                    live.stacks, live.until, live.max_ticks
                ))
            }),
            Patch::Item { count, .. } | Patch::Acts { count, .. } => {
                StateValue::Number(i64::from(*count))
            }
            Patch::Wielded { item, .. } => item.clone().map_or(StateValue::Empty, StateValue::Text),
            Patch::KnownFacts { facts, .. } => StateValue::List(facts.clone()),
            Patch::Busy { until, .. } => until.map_or(StateValue::Empty, number),
            Patch::LastActRound { round, .. } => number(*round),
            Patch::Note { note: Some(_), .. } => StateValue::Bool(true),
            Patch::Note { note: None, .. } | Patch::Visited { .. } | Patch::Record { .. } => {
                StateValue::Empty
            }
        }
    }

    /// What a modifier patch adds to its meter, `None` for any other patch or for a modifier
    /// that is gone.
    pub(crate) fn stacks(&self) -> Option<&[i64]> {
        match self {
            Patch::Modifier { held, .. } => held.as_ref().map(|live| live.stacks.as_slice()),
            _ => None,
        }
    }
}

/// Swap `value` with `map`'s entry for `id`, `None` being no entry.
fn swap_entry<T>(map: &mut BTreeMap<String, T>, id: &str, value: &mut Option<T>) {
    let old = match value.take() {
        Some(new) => map.insert(id.to_string(), new),
        None => map.remove(id),
    };
    *value = old;
}

/// Give `id` an empty live record, or take its record away, and say whether it had one.
fn set_present<T: Default>(records: &mut BTreeMap<String, T>, id: &str, here: bool) -> bool {
    let was = records.contains_key(id);
    if here {
        records.entry(id.to_string()).or_default();
    } else {
        records.remove(id);
    }
    was
}

fn number(value: u64) -> StateValue {
    StateValue::Number(i64::try_from(value).unwrap_or(i64::MAX))
}
