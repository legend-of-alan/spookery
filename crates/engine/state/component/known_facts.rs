//! The facts each actor knows.

use crate::state::change::{self, Change};
use crate::state::component::{Component, Mut, Ref};
use crate::state::{entity, sort_unique};
use crate::Reason;

/// The facts an actor knows, in order with no repeats.
pub struct KnownFacts;

impl Component for KnownFacts {
    type Value = Vec<String>;
}

impl<'a> Ref<'a, entity::Actors, KnownFacts> {
    /// The facts the actor knows.
    pub fn get(&self) -> Option<&'a [String]> {
        (self.entities.components.known_facts.get(self.id)).map(Vec::as_slice)
    }
}

impl Mut<'_, entity::Actors, KnownFacts> {
    /// The actor knows `facts` from now on, kept in order with no repeats.
    pub fn learn(&mut self, facts: &[String]) -> Result<(), Reason> {
        self.check_exists()?;
        let held = self.writer.components.known_facts.get(self.id).cloned();
        let held = held.unwrap_or_default();
        if facts.iter().all(|fact| held.contains(fact)) {
            return Ok(());
        }
        let mut next = held.clone();
        next.extend_from_slice(facts);
        sort_unique(&mut next);
        let key = change::Key::ActorKnownFact(self.id.to_string());
        let (before, after) = (change::Value::List(held), change::Value::List(next.clone()));
        let reports = [Change::new(key, before, after).into()];
        let (known_facts, next) = (&mut self.writer.components.known_facts, Some(next));
        known_facts.set(self.writer.transaction, self.id.clone(), next, reports)
    }
}
