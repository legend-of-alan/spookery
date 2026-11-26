//! Meter bounds from the pack, and the live value on its entity.

use serde::{Deserialize, Serialize};

use crate::ecs::change;
use crate::ecs::component::{modifier, Component, Mut, Ref};
use crate::ecs::entity::{self, in_collection, Id};
use crate::ecs::prelude::*;
use crate::ecs::{Reader, Save, Writer};
use crate::Reason;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Descriptor {
    /// What a player reads for the meter, or empty when the pack gives none.
    pub name: String,
    pub min: i64,
    pub base_cap: i64,
    pub max: i64,
    pub base: i64,
    /// Kept off the players' views.
    pub hidden: bool,
    /// Who carries it, from the file that declares it. `None` is every actor.
    pub carrier: Option<entity::Id>,
}

impl Descriptor {
    /// Whether `id` carries this meter.
    pub fn is_on(&self, id: entity::IdRef<'_>) -> bool {
        match &self.carrier {
            None => matches!(id, entity::IdRef::Actor(_)),
            Some(carrier) => carrier.as_ref() == id,
        }
    }

    /// Whether every actor, or one actor, carries this meter.
    pub fn is_actor_meter(&self) -> bool {
        matches!(self.carrier, None | Some(entity::Id::Actor(_)))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Meter {
    pub base: i64,
    pub offset: i64,
}

impl Meter {
    /// The meter a carrier with no save row reads: `desc`'s base, with no offset.
    pub fn at_base(desc: &Descriptor) -> Meter {
        Meter {
            base: desc.base,
            offset: 0,
        }
    }
}

/// A change to a meter's offset. A journal line writes it as `"write":"offset","delta":<n>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "write", content = "delta", rename_all = "snake_case")]
pub enum Write {
    Offset(i64),
}

/// `base` is inside `[min, base_cap]`, and `base + offset` fits in `i64`.
/// A sum outside `[min, max]` still returns true. `value` clamps that.
pub(crate) fn is_storable(desc: &Descriptor, base: i64, offset: i64) -> bool {
    (desc.min..=desc.base_cap).contains(&base) && base.checked_add(offset).is_some()
}

/// `base + offset`, plus what the entity's modifiers add to this meter, clamped to the bounds.
pub(crate) fn value(desc: &Descriptor, live: &Meter, modifiers: i64) -> Result<i64, Reason> {
    let sum = live
        .base
        .checked_add(live.offset)
        .and_then(|sum| sum.checked_add(modifiers))
        .ok_or(Reason::OutOfRange)?;
    Ok(sum.clamp(desc.min, desc.max))
}

/// `held` with its offset moved by `delta` inside `desc`'s bounds.
fn shift(held: Meter, desc: &Descriptor, delta: i64) -> Result<Meter, Reason> {
    let offset = held.offset.checked_add(delta).ok_or(Reason::OutOfRange)?;
    if !is_storable(desc, held.base, offset) {
        return Err(Reason::OutOfRange);
    }
    Ok(Meter { offset, ..held })
}

impl Component for Meter {
    type Value = Meter;
    const ID: &'static str = "meter";
}

impl<E: entity::Has<Meter, Store = entity::MapStore<E, Meter>>> Ref<'_, E, Meter> {
    /// Meter `entry_id` as the entity holds it: its save row, or `Meter::at_base` when it has
    /// none. `None` when the entity does not carry the meter.
    pub fn get_or_base(&self, entry_id: &str) -> Option<Meter> {
        let desc = (self.entities.canon).meter_def(self.id.as_id_ref(), entry_id)?;
        let held = self.get(entry_id).copied();
        (self.entities.contains(self.id)).then(|| held.unwrap_or(Meter::at_base(desc)))
    }
}

impl<E: entity::Has<Meter, Store = entity::MapStore<E, Meter>>> Mut<'_, E, Meter> {
    /// Move meter `entry_id` by `delta` inside its bounds. A meter back at the pack's base leaves the
    /// save. It reports the meter's key with the meter it replaced; the change is read, as rules
    /// read the meter, when the changes are taken.
    pub fn shift(&mut self, entry_id: &str, delta: i64) -> Result<(), Reason> {
        self.check_exists()?;
        let id = self.id.as_id_ref();
        let desc = (self.writer.canon.meter_def(id, entry_id)).ok_or(Reason::UnknownId)?;
        let pack_base = Meter::at_base(desc);
        let entry = (self.id.clone(), entry_id.to_string());
        let held = self.store().get(&entry).copied().unwrap_or(pack_base);
        let next = shift(held, desc, delta)?;
        let report = change::Report::Meter(self.key(Some(entry_id)), Some(held));
        let next = (next != pack_base).then_some(next);
        let (store, transaction) = self.store_mut();
        store.set(transaction, entry, next, [report])
    }

    /// Give the entity meter `entry_id` as `meter` says, `None` taking it off, outside a round's
    /// rules: migrate and fixtures. It reports as `shift` does. Taking a meter off checks no
    /// owner: the entry is its own evidence.
    pub fn set(&mut self, entry_id: &str, meter: Option<Meter>) -> Result<(), Reason> {
        if meter.is_some() {
            self.check_exists()?;
        }
        let entry = (self.id.clone(), entry_id.to_string());
        let held = self.store().get(&entry).copied();
        let report = change::Report::Meter(self.key(Some(entry_id)), held);
        let (store, transaction) = self.store_mut();
        store.set(transaction, entry, meter, [report])
    }
}

impl Ref<'_, Save, Meter> {
    /// Meter `entry_id` as rolls, conditions and views read it: the stored meter inside the pack's
    /// bounds, plus an actor's modifiers (`modifier::value`). `UNKNOWN_ID` when the entity does
    /// not carry the meter.
    pub fn value(&self, entry_id: &str) -> Result<i64, Reason> {
        let (save, canon) = (self.entities.components, self.entities.canon);
        match self.id {
            Id::Actor(actor_id) => {
                modifier::value(Reader::new(&save.actors, canon), actor_id, entry_id)
            }
            id => {
                let desc = canon
                    .meter_def(id.as_ref(), entry_id)
                    .ok_or(Reason::UnknownId);
                let held = in_collection!(id, [&save], |entities, id| {
                    Reader::new(entities, canon)
                        .get::<Meter>(id)
                        .get_or_base(entry_id)
                });
                value(desc?, &held.ok_or(Reason::UnknownId)?, 0)
            }
        }
    }
}

impl Mut<'_, Save, Meter> {
    /// As on its collection.
    pub fn shift(&mut self, entry_id: &str, delta: i64) -> Result<(), Reason> {
        let (transaction, canon) = (&mut *self.writer.transaction, self.writer.canon);
        in_collection!(self.id, [&mut self.writer.components], |entities, id| {
            Writer::new(entities, transaction, canon)
                .get_mut::<Meter>(id)
                .shift(entry_id, delta)
        })
    }

    /// As on its collection: migrate and fixtures.
    pub fn set(&mut self, entry_id: &str, meter: Option<Meter>) -> Result<(), Reason> {
        let (transaction, canon) = (&mut *self.writer.transaction, self.writer.canon);
        in_collection!(self.id, [&mut self.writer.components], |entities, id| {
            Writer::new(entities, transaction, canon)
                .get_mut::<Meter>(id)
                .set(entry_id, meter)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vigor() -> Descriptor {
        Descriptor {
            name: String::new(),
            min: 0,
            base_cap: 100,
            max: 110,
            base: 1,
            hidden: false,
            carrier: None,
        }
    }

    #[test]
    fn stored_accepts_a_sum_past_max_and_current_clamps() {
        let desc = vigor();
        assert!(is_storable(&desc, 100, 20));
        let live = Meter {
            base: 100,
            offset: 20,
        };
        assert_eq!(value(&desc, &live, 0).unwrap(), 110);
        assert_eq!(value(&desc, &live, -15).unwrap(), 105);
    }

    #[test]
    fn stored_rejects_a_sum_that_does_not_fit() {
        assert!(!is_storable(&vigor(), 3, i64::MAX));
    }
}
