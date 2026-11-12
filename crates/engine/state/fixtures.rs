//! The actor lifecycle only fixtures need: production has no path that adds or removes an actor.
//! Each component goes through its production writer where one exists.

use super::component::{self, Meter};
use super::id::ActorId;
use super::{entity, nested_map, Act, ActorKind, Writer};
use crate::Reason;

impl Writer<'_, entity::Actors> {
    /// Add `actor`, of `kind`, standing at `place`: the three stores every actor has, together.
    pub fn add_actor(&mut self, actor: &ActorId, kind: ActorKind, place: &str) {
        let (components, transaction) = (&mut *self.components, &mut *self.transaction);
        let added = components
            .kind
            .set(transaction, actor.clone(), Some(kind), []);
        added.expect("a write that reports nothing is never refused");
        components
            .place
            .put(transaction, actor, Some(place.to_string()));
        components
            .acts
            .put(transaction, actor, Some(Act::default()));
    }

    /// Add `to`, standing at `place`, with everything `from` has.
    pub fn copy_actor(&mut self, from: &ActorId, to: &ActorId, place: &str) -> Result<(), Reason> {
        // Every store, with no `..`: a store added later must be copied here to compile.
        let entity::Actors {
            kind,
            place: _,
            wielded,
            known_facts,
            visited,
            acts,
            items,
            meters,
            modifiers,
            flags,
        } = (*self.components).clone();
        self.add_actor(to, *kind.get(from).ok_or(Reason::UnknownId)?, place);
        let act = acts.get(from).cloned();
        self.components.acts.put(self.transaction, to, act);
        for (id, count) in nested_map::entries_of(&items, from) {
            (self.get_mut::<component::Item>(to)).shift(id, (*count).into(), None)?;
        }
        self.set_wielded(to, wielded.get(from).map(String::as_str))?;
        self.learn(to, known_facts.get(from).map_or(&[], Vec::as_slice))?;
        for (place, exits) in nested_map::entries_of(&visited, from) {
            (self.get_mut::<component::Visit>(to)).set(place, exits.clone())?;
        }
        for (key, meter) in nested_map::entries_of(&meters, from) {
            self.get_mut::<Meter>(to).set(key, Some(meter.clone()))?;
        }
        for (id, modifier) in nested_map::entries_of(&modifiers, from) {
            (self.get_mut::<component::Modifier>(to)).set(id, Some(modifier.clone()))?;
        }
        for (key, ()) in nested_map::entries_of(&flags, from) {
            self.get_mut::<component::Flag>(to).set(key, true)?;
        }
        Ok(())
    }

    /// Take `actor` off every store, `kind` last.
    pub fn remove_actor(&mut self, actor: &ActorId) -> Result<(), Reason> {
        // Every store, with no `..`: a store added later must be emptied here to compile.
        let entity::Actors {
            kind: _,
            place: _,
            wielded: _,
            known_facts: _,
            visited,
            acts: _,
            items,
            meters,
            modifiers,
            flags,
        } = (*self.components).clone();
        self.set_wielded(actor, None)?;
        for (id, count) in nested_map::entries_of(&items, actor) {
            (self.get_mut::<component::Item>(actor)).shift(id, -i64::from(*count), None)?;
        }
        for (id, _) in nested_map::entries_of(&modifiers, actor) {
            self.get_mut::<component::Modifier>(actor).set(id, None)?;
        }
        for (key, _) in nested_map::entries_of(&meters, actor) {
            self.get_mut::<Meter>(actor).set(key, None)?;
        }
        for (key, ()) in nested_map::entries_of(&flags, actor) {
            self.get_mut::<component::Flag>(actor).set(key, false)?;
        }
        // No production writer takes these off: production removes no actor.
        let (components, transaction) = (&mut *self.components, &mut *self.transaction);
        components
            .known_facts
            .set(transaction, actor.clone(), None, [])?;
        for (place, _) in nested_map::entries_of(&visited, actor) {
            let entry = (actor.clone(), place.to_string());
            components.visited.set(transaction, entry, None, [])?;
        }
        components.acts.put(transaction, actor, None);
        components.place.put(transaction, actor, None);
        components.kind.set(transaction, actor.clone(), None, [])
    }
}
