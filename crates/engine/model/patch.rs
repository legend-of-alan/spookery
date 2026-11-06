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
    /// Put this patch's value into `state` and give back the patch that holds the value it
    /// replaced: swapping that one in puts the save back. A record the field sits in that is
    /// not there is `UNKNOWN_ID`, and nothing is written.
    pub(crate) fn swap(self, state: &mut State) -> Result<Patch, Reason> {
        Ok(match self {
            Patch::ActorPlace { actor, place } => {
                let old = mem::replace(&mut state.actor_mut(&actor)?.place, place);
                Patch::ActorPlace { actor, place: old }
            }
            Patch::Flag { holder, key, on } => {
                let old = state.has_flag(&holder, &key);
                if old != on {
                    let (flags, entry) = state.flags_mut(&holder, &key)?;
                    if on {
                        flags.push(entry);
                    } else {
                        flags.retain(|flag| *flag != entry);
                    }
                }
                Patch::Flag {
                    holder,
                    key,
                    on: old,
                }
            }
            Patch::Meter { holder, key, live } => {
                let meters = state.meters_mut(&holder)?;
                let old = mem::replace(meters.get_mut(&key).ok_or(Reason::UnknownId)?, live);
                Patch::Meter {
                    holder,
                    key,
                    live: old,
                }
            }
            Patch::Modifier { actor, id, held } => {
                let modifiers = &mut state.actor_mut(&actor)?.modifiers;
                let old = put(modifiers, &id, held);
                Patch::Modifier {
                    actor,
                    id,
                    held: old,
                }
            }
            Patch::Item { holder, id, count } => {
                let counts = match &holder {
                    Holder::Actor(actor) => &mut state.actor_mut(actor)?.inventory,
                    Holder::Place(place) => {
                        &mut state.places.get_mut(place).ok_or(Reason::UnknownId)?.items
                    }
                    Holder::Story(_) | Holder::Exit(..) | Holder::World => {
                        return Err(Reason::Schema)
                    }
                };
                let old = put(counts, &id, (count > 0).then_some(count));
                Patch::Item {
                    holder,
                    id,
                    count: old.unwrap_or(0),
                }
            }
            Patch::Wielded { actor, item } => {
                let old = mem::replace(&mut state.actor_mut(&actor)?.wielded, item);
                Patch::Wielded { actor, item: old }
            }
            Patch::KnownFacts { actor, facts } => {
                let old = mem::replace(&mut state.actor_mut(&actor)?.known_facts, facts);
                Patch::KnownFacts { actor, facts: old }
            }
            Patch::Busy {
                actor,
                until,
                pending,
            } => {
                let act = &mut state.actor_mut(&actor)?.act;
                let old_until = mem::replace(&mut act.busy_until, until);
                let old_pending = mem::replace(&mut act.pending, pending);
                Patch::Busy {
                    actor,
                    until: old_until,
                    pending: old_pending,
                }
            }
            Patch::LastActRound { actor, round } => {
                let act = &mut state.actor_mut(&actor)?.act;
                let old = mem::replace(&mut act.last_act_round, round);
                Patch::LastActRound { actor, round: old }
            }
            Patch::Acts {
                actor,
                counter,
                count,
            } => {
                let act = &mut state.actor_mut(&actor)?.act;
                let old = mem::replace(counter.count_mut(act), count);
                Patch::Acts {
                    actor,
                    counter,
                    count: old,
                }
            }
            Patch::Visited {
                actor,
                place,
                exits,
            } => {
                let old = put(&mut state.actor_mut(&actor)?.visited, &place, exits);
                Patch::Visited {
                    actor,
                    place,
                    exits: old,
                }
            }
            Patch::Note { id, note } => {
                let at = state.notes.iter().position(|left| left.id == id);
                let old = at.map(|at| state.notes.remove(at));
                state.notes.extend(note);
                Patch::Note { id, note: old }
            }
            Patch::Record { holder, here } => {
                let old = match &holder {
                    Holder::Place(id) => set_present(&mut state.places, id, here),
                    Holder::Story(id) => set_present(&mut state.stories, id, here),
                    Holder::Actor(_) | Holder::Exit(..) | Holder::World => {
                        return Err(Reason::Schema)
                    }
                };
                Patch::Record { holder, here: old }
            }
        })
    }

    /// The key this patch writes and the value that key reads with this patch's value in
    /// place, or `None` for a write no key answers for. A meter reads through the pack, with
    /// the holder's modifiers as `state` has them.
    pub(crate) fn read(&self, canon: &Canon, state: &State) -> Option<(StateKey, StateValue)> {
        Some(match self {
            Patch::ActorPlace { actor, place } => (
                StateKey::ActorPlace(actor.clone()),
                StateValue::Text(place.clone()),
            ),
            Patch::Flag { holder, key, on } => (StateKey::flag(holder, key), StateValue::Bool(*on)),
            Patch::Meter { holder, key, live } => {
                let value = canon
                    .meter_def(holder, key)
                    .zip(state.modifier_sum(holder, key).ok())
                    .and_then(|(def, added)| meter::current(def, live, added).ok());
                (
                    StateKey::meter(holder, key)?,
                    value.map_or(StateValue::Empty, StateValue::Number),
                )
            }
            Patch::Modifier { actor, id, held } => (
                StateKey::ActorModifier(actor.clone(), id.clone()),
                held.as_ref().map_or(StateValue::Empty, |live| {
                    StateValue::Text(format!(
                        "{:?} until {} max_ticks {}",
                        live.stacks, live.until, live.max_ticks
                    ))
                }),
            ),
            Patch::Item { holder, id, count } => (
                StateKey::item(holder, id)?,
                StateValue::Number(i64::from(*count)),
            ),
            Patch::Wielded { actor, item } => (
                StateKey::ActorWielded(actor.clone()),
                item.clone().map_or(StateValue::Empty, StateValue::Text),
            ),
            Patch::KnownFacts { actor, facts } => (
                StateKey::ActorKnownFact(actor.clone()),
                StateValue::List(facts.clone()),
            ),
            Patch::Busy { actor, until, .. } => (
                StateKey::ActorBusy(actor.clone()),
                until.map_or(StateValue::Empty, number),
            ),
            Patch::LastActRound { actor, round } => {
                (StateKey::LastActRound(actor.clone()), number(*round))
            }
            Patch::Acts {
                actor,
                counter,
                count,
            } => (
                counter.key(actor.clone()),
                StateValue::Number(i64::from(*count)),
            ),
            Patch::Note { id, note } => (
                StateKey::Note(id.clone()),
                if note.is_some() {
                    StateValue::Bool(true)
                } else {
                    StateValue::Empty
                },
            ),
            Patch::Visited { .. } | Patch::Record { .. } => return None,
        })
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

/// Insert `value` under `id`, or remove the entry for `None`, and give back what was there.
fn put<T>(map: &mut BTreeMap<String, T>, id: &str, value: Option<T>) -> Option<T> {
    match value {
        Some(value) => map.insert(id.to_string(), value),
        None => map.remove(id),
    }
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
