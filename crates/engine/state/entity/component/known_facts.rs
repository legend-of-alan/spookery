//! The facts each actor knows.

use std::collections::BTreeSet;

use crate::state::change::{self, Change};
use crate::state::component::{Component, Mut, Ref};
use crate::state::entity;
use crate::Reason;

/// The facts an actor knows, in order with no repeats.
pub struct KnownFacts;

impl Component for KnownFacts {
    type Value = BTreeSet<String>;
    const NAME: &'static str = "known_facts";
}

impl<'a> Ref<'a, entity::Actors, KnownFacts> {
    /// The facts the actor knows.
    pub fn get(&self) -> Option<&'a BTreeSet<String>> {
        self.store().get(self.id)
    }
}

impl Mut<'_, entity::Actors, KnownFacts> {
    /// The actor knows `facts` from now on.
    pub fn learn<'f>(&mut self, facts: impl IntoIterator<Item = &'f String>) -> Result<(), Reason> {
        self.check_exists()?;
        let held = self.store().get(self.id).cloned().unwrap_or_default();
        let mut next = held.clone();
        next.extend(facts.into_iter().cloned());
        if next == held {
            return Ok(());
        }
        let key = self.key(None);
        let before = change::Value::List(held.into_iter().collect());
        let after = change::Value::List(next.iter().cloned().collect());
        let reports = [Change::new(key, before, after).into()];
        let (id, (known_facts, transaction)) = (self.id.clone(), self.store_mut());
        known_facts.set(transaction, id, Some(next), reports)
    }
}
