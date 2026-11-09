//! The actor lifecycle only fixtures need: production has no path that adds or removes an actor.
//! Each component goes through its production writer where one exists.

use super::id::ActorId;
use super::{nested_map, Act, ActorComponents, ActorKind, Writer};
use crate::canon::Canon;
use crate::Reason;

impl Writer<'_, ActorComponents> {
    /// Add `actor`, of `kind`, standing at `place`: the three stores every actor has, together.
    pub fn add_actor(&mut self, actor: &ActorId, kind: ActorKind, place: &str) {
        let Writer { part, transaction } = self;
        let added = part.kind.set(transaction, actor.clone(), Some(kind), []);
        added.expect("a write that reports nothing is never refused");
        part.place.put(transaction, actor, Some(place.to_string()));
        part.acts.put(transaction, actor, Some(Act::default()));
    }

    /// Add `to`, standing at `place`, with everything `from` has.
    pub fn copy_actor(
        &mut self,
        canon: &Canon,
        from: &ActorId,
        to: &ActorId,
        place: &str,
    ) -> Result<(), Reason> {
        // Every store, with no `..`: a store added later must be copied here to compile.
        let ActorComponents {
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
        } = (*self.part).clone();
        self.add_actor(to, *kind.get(from).ok_or(Reason::UnknownId)?, place);
        let act = acts.get(from).cloned();
        self.part.acts.put(self.transaction, to, act);
        for (id, count) in nested_map::entries_of(&items, from) {
            self.shift_item(to, id, (*count).into(), None)?;
        }
        self.set_wielded(to, wielded.get(from).map(String::as_str))?;
        self.learn(to, known_facts.get(from).map_or(&[], Vec::as_slice))?;
        for (place, exits) in nested_map::entries_of(&visited, from) {
            self.note_exits(to, place, exits.clone())?;
        }
        for (key, meter) in nested_map::entries_of(&meters, from) {
            self.set_meter(canon, to, key, Some(meter.clone()))?;
        }
        for (id, modifier) in nested_map::entries_of(&modifiers, from) {
            self.set_modifier(canon, to, id, Some(modifier.clone()))?;
        }
        for (_, key) in flags.iter().filter(|(owner, _)| owner == from) {
            self.set_flag(to, key, true)?;
        }
        Ok(())
    }

    /// Take `actor` off every store, `kind` last.
    pub fn remove_actor(&mut self, canon: &Canon, actor: &ActorId) -> Result<(), Reason> {
        // Every store, with no `..`: a store added later must be emptied here to compile.
        let ActorComponents {
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
        } = (*self.part).clone();
        self.set_wielded(actor, None)?;
        for (id, count) in nested_map::entries_of(&items, actor) {
            self.shift_item(actor, id, -i64::from(*count), None)?;
        }
        for (id, _) in nested_map::entries_of(&modifiers, actor) {
            self.set_modifier(canon, actor, id, None)?;
        }
        for (key, _) in nested_map::entries_of(&meters, actor) {
            self.set_meter(canon, actor, key, None)?;
        }
        for (_, key) in flags.iter().filter(|(owner, _)| owner == actor) {
            self.set_flag(actor, key, false)?;
        }
        // No production writer takes these off: production removes no actor.
        let Writer { part, transaction } = self;
        part.known_facts.set(transaction, actor.clone(), None, [])?;
        for (place, _) in nested_map::entries_of(&visited, actor) {
            let entry = (actor.clone(), place.to_string());
            part.visited.set(transaction, entry, None, [])?;
        }
        part.acts.put(transaction, actor, None);
        part.place.put(transaction, actor, None);
        part.kind.set(transaction, actor.clone(), None, [])
    }
}
