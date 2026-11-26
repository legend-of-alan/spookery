//! Meter bounds from the pack, and the live value on its entity.

use serde::{Deserialize, Serialize};

use crate::ecs::change;
use crate::ecs::component::{modifier, Component, Mut, Ref};
use crate::ecs::entity::{self, in_collection, Id};
use crate::ecs::prelude::*;
use crate::ecs::{Reader, Save, Writer};
use crate::Reason;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Def {
    /// What a player reads for the meter, or empty when the pack gives none.
    pub name: String,
    pub min: i64,
    pub base_cap: i64,
    pub max: i64,
    pub base: i64,
    /// Kept off the players' views.
    pub hidden: bool,
    /// The entity that carries it, from the file that declares it. `None` is every actor.
    pub entity_id: Option<entity::Id>,
}

impl Def {
    /// Whether `id` carries this meter.
    pub fn is_on(&self, id: entity::IdRef<'_>) -> bool {
        match &self.entity_id {
            None => matches!(id, entity::IdRef::Actor(_)),
            Some(entity_id) => entity_id.as_ref() == id,
        }
    }

    /// Whether every actor, or one actor, carries this meter.
    pub fn is_actor_meter(&self) -> bool {
        matches!(self.entity_id, None | Some(entity::Id::Actor(_)))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Meter {
    pub base: i64,
    pub offset: i64,
}

impl Meter {
    /// The meter an entity with no save row reads: `def`'s base, with no offset.
    pub fn at_base(def: &Def) -> Meter {
        Meter {
            base: def.base,
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
pub(crate) fn is_storable(def: &Def, base: i64, offset: i64) -> bool {
    (def.min..=def.base_cap).contains(&base) && base.checked_add(offset).is_some()
}

/// `base + offset`, plus what the entity's modifiers add to this meter, clamped to the bounds.
pub(crate) fn value(def: &Def, live: &Meter, modifiers: i64) -> Result<i64, Reason> {
    let sum = live
        .base
        .checked_add(live.offset)
        .and_then(|sum| sum.checked_add(modifiers))
        .ok_or(Reason::OutOfRange)?;
    Ok(sum.clamp(def.min, def.max))
}

/// `meter` with its offset moved by `delta` inside `def`'s bounds.
fn shift(meter: Meter, def: &Def, delta: i64) -> Result<Meter, Reason> {
    let offset = meter.offset.checked_add(delta).ok_or(Reason::OutOfRange)?;
    if !is_storable(def, meter.base, offset) {
        return Err(Reason::OutOfRange);
    }
    Ok(Meter { offset, ..meter })
}

impl Component for Meter {
    type Value = Meter;
    const ID: &'static str = "meter";
}

impl<E: entity::Has<Meter, Store = entity::MapStore<E, Meter>>> Ref<'_, E, Meter> {
    /// Meter `entry_id` as the entity holds it: its save row, or `Meter::at_base` when it has
    /// none. `None` when the entity does not carry the meter.
    pub fn get_or_base(&self, entry_id: &str) -> Option<Meter> {
        let def = (self.entities.canon).meter_def(self.id.as_id_ref(), entry_id)?;
        let meter = self.get(entry_id).copied();
        (self.entities.contains(self.id)).then(|| meter.unwrap_or(Meter::at_base(def)))
    }
}

impl<E: entity::Has<Meter, Store = entity::MapStore<E, Meter>>> Mut<'_, E, Meter> {
    /// Move meter `entry_id` by `delta` inside its bounds. A meter back at the pack's base leaves the
    /// save. It reports the meter's key with the meter it replaced; the change is read, as rules
    /// read the meter, when the changes are taken.
    pub fn shift(&mut self, entry_id: &str, delta: i64) -> Result<(), Reason> {
        self.check_exists()?;
        let id = self.id.as_id_ref();
        let def = (self.writer.canon.meter_def(id, entry_id)).ok_or(Reason::UnknownId)?;
        let pack_base = Meter::at_base(def);
        let entry = (self.id.clone(), entry_id.to_string());
        let meter = self.store().get(&entry).copied().unwrap_or(pack_base);
        let next = shift(meter, def, delta)?;
        let report = change::Report::Meter(self.key(Some(entry_id)), Some(meter));
        let next = (next != pack_base).then_some(next);
        let (store, transaction) = self.store_mut();
        store.set(transaction, entry, next, [report])
    }

    /// Give the entity meter `entry_id` as `meter` says, `None` taking it off, outside a round's
    /// rules: migrate and fixtures. It reports as `shift` does. Taking a meter off checks no
    /// parent: the entry is its own evidence.
    pub fn set(&mut self, entry_id: &str, meter: Option<Meter>) -> Result<(), Reason> {
        if meter.is_some() {
            self.check_exists()?;
        }
        let entry = (self.id.clone(), entry_id.to_string());
        let previous = self.store().get(&entry).copied();
        let report = change::Report::Meter(self.key(Some(entry_id)), previous);
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
                let def = canon
                    .meter_def(id.as_ref(), entry_id)
                    .ok_or(Reason::UnknownId);
                let meter = in_collection!(id, [&save], |entities, id| {
                    Reader::new(entities, canon)
                        .get::<Meter>(id)
                        .get_or_base(entry_id)
                });
                value(def?, &meter.ok_or(Reason::UnknownId)?, 0)
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

    fn vigor() -> Def {
        Def {
            name: String::new(),
            min: 0,
            base_cap: 100,
            max: 110,
            base: 1,
            hidden: false,
            entity_id: None,
        }
    }

    #[test]
    fn stored_accepts_a_sum_past_max_and_current_clamps() {
        let def = vigor();
        assert!(is_storable(&def, 100, 20));
        let live = Meter {
            base: 100,
            offset: 20,
        };
        assert_eq!(value(&def, &live, 0).unwrap(), 110);
        assert_eq!(value(&def, &live, -15).unwrap(), 105);
    }

    #[test]
    fn stored_rejects_a_sum_that_does_not_fit() {
        assert!(!is_storable(&vigor(), 3, i64::MAX));
    }
}
