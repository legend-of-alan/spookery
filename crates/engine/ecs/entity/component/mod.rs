//! What an entity holds, one module per component. A marker type names each component;
//! `Reader::get` borrows one entity's share of its store, and `Writer::get_mut` writes it.

pub(in crate::ecs) mod act;
pub(in crate::ecs) mod control_type;
pub(in crate::ecs) mod exit;
pub(in crate::ecs) mod flag;
pub(in crate::ecs) mod item;
pub(in crate::ecs) mod known_facts;
pub(crate) mod meter;
pub(crate) mod modifier;
pub(in crate::ecs) mod place;
pub(in crate::ecs) mod visit;
pub(in crate::ecs) mod wielded_item;

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::marker::PhantomData;

pub(crate) use act::ActCounter;
pub use act::{Act, PendingAction};
pub use control_type::ControlType;
pub use exit::OpenExit;
pub use flag::Flag;
pub use item::Item;
pub use known_facts::KnownFacts;
pub use meter::Meter;
pub use modifier::Modifier;
pub use place::Place;
pub use visit::Visit;
pub use wielded_item::WieldedItem;

use crate::ecs::change::{self, Change};
use crate::ecs::prelude::*;
use crate::ecs::{entity, nested_map, Reader, Transaction, Writer};
use crate::Reason;

/// One component. The type that implements it is the component in code, and `Value` is what an
/// entity holds under each entry id.
pub trait Component {
    type Value: PartialEq + Debug + 'static;
    /// The component's id, as change keys and a player's change view spell it. `Act`'s keys
    /// name the part of it that moved instead.
    const ID: &'static str;
}

/// The map a store keeps component `C` in, for a collection whose ids are `Id`.
pub type Map<Id, C> = BTreeMap<(Id, String), <C as Component>::Value>;

/// One entity's component `C`, borrowed from its collection's store. Nothing is copied.
pub struct Ref<'a, E: entity::Collection, C: Component> {
    entities: Reader<'a, E>,
    id: &'a E::Id,
    component: PhantomData<C>,
}

impl<'a, E: entity::Collection, C: Component> Ref<'a, E, C> {
    pub(in crate::ecs) fn new(entities: Reader<'a, E>, id: &'a E::Id) -> Self {
        Ref {
            entities,
            id,
            component: PhantomData,
        }
    }
}

/// The component's store: only the component views under `component` reach it.
impl<'a, E: entity::Has<C>, C: Component> Ref<'a, E, C> {
    fn store(&self) -> &'a E::Store {
        self.entities.components.store()
    }
}

/// A component an entity holds many of, one under each id.
impl<'a, E, C> Ref<'a, E, C>
where
    E: entity::Has<C, Store = entity::MapStore<E, C>>,
    C: Component,
{
    /// What the entity holds under `entry_id`.
    pub fn get(&self, entry_id: &str) -> Option<&'a C::Value> {
        nested_map::get(self.store(), self.id, entry_id)
    }

    pub fn contains(&self, entry_id: &str) -> bool {
        self.get(entry_id).is_some()
    }

    /// Each id the entity holds with its value, in id order.
    pub fn iter(&self) -> impl Iterator<Item = (&'a str, &'a C::Value)> {
        nested_map::entries_of(self.store(), self.id)
    }

    /// The same, each after the entity's id: a query's entities, walked one value at a time.
    pub fn iter_with_id(&self) -> impl Iterator<Item = (&'a E::Id, &'a str, &'a C::Value)> {
        let id = self.id;
        self.iter()
            .map(move |(entry_id, value)| (id, entry_id, value))
    }
}

/// One entity's component `C`, to write. Every write is one `undo::Store::set`, with what it
/// reports.
pub struct Mut<'a, E: entity::Collection, C: Component> {
    writer: Writer<'a, E>,
    id: &'a E::Id,
    component: PhantomData<C>,
}

impl<'a, E: entity::Collection, C: Component> Mut<'a, E, C> {
    pub(in crate::ecs) fn new(writer: Writer<'a, E>, id: &'a E::Id) -> Self {
        Mut {
            writer,
            id,
            component: PhantomData,
        }
    }

    /// The key a write of the entity's `entry_id` reports: `None` when it holds one value of `C`.
    fn key(&self, entry_id: Option<&str>) -> change::Key {
        change::Key::new(C::ID, self.id.as_id_ref(), entry_id)
    }

    /// `UNKNOWN_ID` unless the entity exists: a value goes only on one that does.
    fn check_exists(&self) -> Result<(), Reason> {
        let exists = (self.writer.components).contains(self.writer.canon, self.id);
        exists.then_some(()).ok_or(Reason::UnknownId)
    }
}

/// The component's store, and the round's transaction its writes go through: only the
/// component views under `component` reach them.
impl<E: entity::Has<C>, C: Component> Mut<'_, E, C> {
    fn store(&self) -> &E::Store {
        self.writer.components.store()
    }

    fn store_mut(&mut self) -> (&mut E::Store, &mut Transaction) {
        (self.writer.components.store_mut(), self.writer.transaction)
    }
}

/// A component that is there or not, such as a flag: every one is set the same way.
impl<E, C> Mut<'_, E, C>
where
    E: entity::Has<C, Store = entity::MapStore<E, C>>,
    C: Component<Value = ()>,
{
    /// Put `entry_id` on or take it off: set a flag, or open or shut a door.
    pub fn set(&mut self, entry_id: &str, on: bool) -> Result<(), Reason> {
        self.check_exists()?;
        let (before, after) = (change::Value::Bool(!on), change::Value::Bool(on));
        let change = Change::new(self.key(Some(entry_id)), before, after);
        let entry = (self.id.clone(), entry_id.to_string());
        let (store, transaction) = self.store_mut();
        store.set(transaction, entry, on.then_some(()), [change.into()])
    }
}

/// A component's text as a change value, `Empty` when there is none.
fn text_value(held: Option<impl AsRef<str>>) -> change::Value {
    held.map_or(change::Value::Empty, |held| {
        change::Value::Text(held.as_ref().to_string())
    })
}
