//! The actor lifecycle only fixtures need: production has no path that adds or removes an actor.
//! Each component goes through its production writer where one exists.

use crate::ecs::component::{self, Meter};
use crate::ecs::id::ActorId;
use crate::ecs::{entity, nested_map, Writer};
use crate::Reason;

impl Writer<'_, entity::Actors> {
    /// Add `actor`, controlled as `control_type` says, standing at `place`: the two stores every
    /// actor has, together.
    pub fn add_actor(
        &mut self,
        actor: &ActorId,
        control_type: component::ControlType,
        place: &str,
    ) {
        let (components, transaction) = (&mut *self.components, &mut *self.transaction);
        let added = components
            .control_type
            .set(transaction, actor.clone(), Some(control_type), []);
        added.expect("a write that reports nothing is never refused");
        components
            .place
            .put(transaction, actor, Some(place.to_string()));
    }

    /// Add `to`, standing at `place`, with everything `from` has.
    pub fn copy_actor(&mut self, from: &ActorId, to: &ActorId, place: &str) -> Result<(), Reason> {
        let entity::Actors {
            control_type,
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
        self.add_actor(to, *control_type.get(from).ok_or(Reason::UnknownId)?, place);
        let act = acts.get(from).cloned();
        (self.components.acts).set(self.transaction, to.clone(), act, [])?;
        for (id, count) in nested_map::entries_of(&items, from) {
            (self.get_mut::<component::Item>(to)).shift(id, (*count).into(), None)?;
        }
        let wielded = wielded.get(from).map(String::as_str);
        (self.get_mut::<component::WieldedItem>(to)).set(wielded)?;
        (self.get_mut::<component::KnownFacts>(to))
            .learn(known_facts.get(from).into_iter().flatten())?;
        for (place, exits) in nested_map::entries_of(&visited, from) {
            (self.get_mut::<component::Visit>(to)).set(place, exits.clone())?;
        }
        for (key, meter) in nested_map::entries_of(&meters, from) {
            self.get_mut::<Meter>(to).set(key, Some(*meter))?;
        }
        for (id, modifier) in nested_map::entries_of(&modifiers, from) {
            (self.get_mut::<component::Modifier>(to)).set(id, Some(modifier.clone()))?;
        }
        for (key, ()) in nested_map::entries_of(&flags, from) {
            self.get_mut::<component::Flag>(to).set(key, true)?;
        }
        Ok(())
    }

    /// Take `actor` off every store, `control_type` last.
    pub fn remove_actor(&mut self, actor: &ActorId) -> Result<(), Reason> {
        let entity::Actors {
            control_type: _,
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
        (self.get_mut::<component::WieldedItem>(actor)).set(None)?;
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
        let (components, transaction) = (&mut *self.components, &mut *self.transaction);
        components
            .known_facts
            .set(transaction, actor.clone(), None, [])?;
        for (place, _) in nested_map::entries_of(&visited, actor) {
            let entry = (actor.clone(), place.to_string());
            components.visited.set(transaction, entry, None, [])?;
        }
        components.acts.set(transaction, actor.clone(), None, [])?;
        components.place.put(transaction, actor, None);
        (components.control_type).set(transaction, actor.clone(), None, [])
    }
}
