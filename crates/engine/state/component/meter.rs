//! Meter bounds from the pack, and the live value on its entity.

use serde::{Deserialize, Serialize};

use crate::state::change;
use crate::state::component::{modifier, Component, Mut, Ref};
use crate::state::entity::{self, in_collection, Id};
use crate::state::prelude::*;
use crate::state::{Reader, Save, Writer};
use crate::Reason;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MeterDef {
    pub min: i64,
    pub base_cap: i64,
    pub max: i64,
    pub start: i64,
    /// Kept off the players' views.
    pub hidden: bool,
    /// Who carries it, from the file that declares it. `None` is every actor.
    pub carrier: Option<entity::Id>,
}

impl MeterDef {
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Meter {
    pub base: i64,
    pub offset: i64,
}

/// A change to a meter's offset. A journal line writes it as `"write":"offset","delta":<n>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "write", content = "delta", rename_all = "snake_case")]
pub enum MeterWrite {
    Offset(i64),
}

/// `base` is inside `[min, base_cap]`, and `base + offset` fits in `i64`.
/// A sum outside `[min, max]` still returns true. `value` clamps that.
pub fn is_storable(def: &MeterDef, base: i64, offset: i64) -> bool {
    (def.min..=def.base_cap).contains(&base) && base.checked_add(offset).is_some()
}

/// `base + offset`, plus what the entity's modifiers add to this meter, clamped to the bounds.
pub fn value(def: &MeterDef, live: &Meter, modifiers: i64) -> Result<i64, Reason> {
    let sum = live
        .base
        .checked_add(live.offset)
        .and_then(|sum| sum.checked_add(modifiers))
        .ok_or(Reason::OutOfRange)?;
    Ok(sum.clamp(def.min, def.max))
}

/// `held` with its offset moved by `delta` inside `def`'s bounds: `UNKNOWN_ID` when nothing is
/// held.
fn shift(held: Option<&Meter>, def: &MeterDef, delta: i64) -> Result<Meter, Reason> {
    let held = held.ok_or(Reason::UnknownId)?;
    let offset = held.offset.checked_add(delta).ok_or(Reason::OutOfRange)?;
    if !is_storable(def, held.base, offset) {
        return Err(Reason::OutOfRange);
    }
    Ok(Meter { offset, ..*held })
}

impl Component for Meter {
    type Value = Meter;
}

impl<E: entity::Has<Meter>> Ref<'_, E, Meter> {
    /// The stored meter `name` inside the pack's bounds, with no modifiers, even on an actor:
    /// `state.entities()` adds them through `modifier::value`. Issue #549 removes the difference.
    /// `UNKNOWN_ID` when the entity does not carry the meter.
    pub fn value(&self, name: &str) -> Result<i64, Reason> {
        let def = (self.entities.canon).meter_def(self.id.as_id_ref(), name);
        let held = self.get(name).ok_or(Reason::UnknownId);
        value(def.ok_or(Reason::UnknownId)?, held?, 0)
    }
}

impl<E: entity::Has<Meter>> Mut<'_, E, Meter> {
    /// Move meter `name` by `delta` inside its bounds. It reports the meter's key with the
    /// stored meter it replaced; the change is read, as rules read the meter, when the changes
    /// are taken.
    pub fn shift(&mut self, name: &str, delta: i64) -> Result<(), Reason> {
        self.check_exists()?;
        let id = self.id.as_id_ref();
        let def = (self.writer.canon.meter_def(id, name)).ok_or(Reason::UnknownId)?;
        let entry = (self.id.clone(), name.to_string());
        let held = self.writer.components.store().get(&entry);
        let next = shift(held, def, delta)?;
        let report = change::Report::Meter(change::Key::meter(id, name), held.cloned());
        let store = self.writer.components.store_mut();
        store.set(self.writer.transaction, entry, Some(next), [report])
    }

    /// Give the entity meter `name` as `meter` says, `None` taking it off, outside a round's
    /// rules: migrate and fixtures. It reports as `shift` does. Taking a meter off checks no
    /// owner: the entry is its own evidence.
    pub fn set(&mut self, name: &str, meter: Option<Meter>) -> Result<(), Reason> {
        if meter.is_some() {
            self.check_exists()?;
        }
        let entry = (self.id.clone(), name.to_string());
        let held = self.writer.components.store().get(&entry).cloned();
        let report = change::Report::Meter(change::Key::meter(self.id.as_id_ref(), name), held);
        let store = self.writer.components.store_mut();
        store.set(self.writer.transaction, entry, meter, [report])
    }
}

impl Ref<'_, Save, Meter> {
    /// As on its collection, but an actor's adds their modifiers through `modifier::value`, which
    /// `state.actors()` does not. Issue #549 removes the difference.
    pub fn value(&self, name: &str) -> Result<i64, Reason> {
        let (save, canon) = (self.entities.components, self.entities.canon);
        match self.id {
            Id::Actor(actor) => modifier::value(Reader::new(&save.actors, canon), actor, name),
            id => in_collection!(id, [&save], |entities, id| {
                Reader::new(entities, canon).get::<Meter>(id).value(name)
            }),
        }
    }
}

impl Mut<'_, Save, Meter> {
    /// As on its collection.
    pub fn shift(&mut self, name: &str, delta: i64) -> Result<(), Reason> {
        let (transaction, canon) = (&mut *self.writer.transaction, self.writer.canon);
        in_collection!(self.id, [&mut self.writer.components], |entities, id| {
            Writer::new(entities, transaction, canon)
                .get_mut::<Meter>(id)
                .shift(name, delta)
        })
    }

    /// As on its collection: migrate and fixtures.
    pub fn set(&mut self, name: &str, meter: Option<Meter>) -> Result<(), Reason> {
        let (transaction, canon) = (&mut *self.writer.transaction, self.writer.canon);
        in_collection!(self.id, [&mut self.writer.components], |entities, id| {
            Writer::new(entities, transaction, canon)
                .get_mut::<Meter>(id)
                .set(name, meter)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grit() -> MeterDef {
        MeterDef {
            min: 0,
            base_cap: 100,
            max: 110,
            start: 1,
            hidden: false,
            carrier: None,
        }
    }

    #[test]
    fn stored_accepts_a_sum_past_max_and_current_clamps() {
        let def = grit();
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
        assert!(!is_storable(&grit(), 3, i64::MAX));
    }
}
