//! One entity, resolved from the id a rule named, to read or to write. This module is outside
//! `state` on purpose: it sees only `State`'s public getters and handles. The save is the
//! collection of every entity, so a component reads and writes through the same
//! `component::Ref` and `component::Mut` as on a typed collection.

use crate::state::component::{self, Component};
pub use crate::state::entity::{Id, IdRef};
use crate::state::{Reader, Save, State, Writer};

/// One entity, to read: every entity's `Reader` and the id. It borrows the save immutably, so a
/// condition reads through it with no transaction in hand.
#[derive(Clone, Copy)]
pub struct Ref<'a>(Reader<'a, Save>, &'a Id);

impl State {
    /// The entity `id` names, to read.
    pub fn entity<'a>(&'a self, id: &'a Id) -> Ref<'a> {
        Ref(self.entities(), id)
    }
}

impl<'a> Ref<'a> {
    /// The entity's component `C`. Only a component every collection holds has reads here.
    pub fn get<C: Component>(self) -> component::Ref<'a, Save, C> {
        self.0.get(self.1)
    }
}

/// The same entity, to write: every entity's `Writer` and the id.
pub struct Mut<'a>(Writer<'a, Save>, &'a Id);

impl State {
    /// The entity `id` names, to write.
    pub fn entity_mut<'a>(&'a mut self, id: &'a Id) -> Mut<'a> {
        Mut(self.entities_mut(), id)
    }
}

impl<'a> Mut<'a> {
    /// The entity's component `C`. Only a component every collection holds has writes here: an
    /// item goes through `state.actors_mut()` or `state.places_mut()`.
    pub fn get_mut<C: Component>(&mut self) -> component::Mut<'_, Save, C> {
        self.0.get_mut(self.1)
    }
}
