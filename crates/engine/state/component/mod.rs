//! What an entity holds, one module per component. A marker type names each component;
//! `Reader::get` borrows one entity's share of its store, and `Writer::get_mut` writes it.

pub(in crate::state) mod act;
pub(in crate::state) mod control_type;
pub(in crate::state) mod exit;
pub(in crate::state) mod flag;
pub(in crate::state) mod item;
pub(in crate::state) mod known_facts;
pub(crate) mod meter;
pub(crate) mod modifier;
pub(in crate::state) mod place;
pub(in crate::state) mod visit;
pub(in crate::state) mod wielded_item;

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

use crate::state::change::{self, Change};
use crate::state::{entity, nested_map, Reader, Writer};
use crate::Reason;

/// One component. The type that implements it is the component's name in code, and `Value` is
/// what an entity holds under each name.
pub trait Component {
    type Value: PartialEq + Debug + 'static;
}

/// The map a store keeps component `C` in, for a collection whose ids are `Id`.
pub type Map<Id, C> = BTreeMap<(Id, String), <C as Component>::Value>;

/// One entity's component `C`, borrowed from its collection's store. Nothing is copied.
pub struct Ref<'a, E: entity::Entities, C: Component> {
    entities: Reader<'a, E>,
    id: &'a E::Id,
    component: PhantomData<C>,
}

impl<'a, E: entity::Entities, C: Component> Ref<'a, E, C> {
    pub(in crate::state) fn new(entities: Reader<'a, E>, id: &'a E::Id) -> Self {
        Ref {
            entities,
            id,
            component: PhantomData,
        }
    }
}

/// A component an entity holds many of, one under each name.
impl<'a, E: entity::Has<C>, C: Component> Ref<'a, E, C> {
    /// What the entity holds under `name`.
    pub fn get(&self, name: &str) -> Option<&'a C::Value> {
        self.entities.components.get(self.id, name)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.get(name).is_some()
    }
}

impl<'a, E: entity::Stores<C>, C: Component> Ref<'a, E, C> {
    /// Each name the entity holds with its value, in name order.
    pub fn iter(&self) -> impl Iterator<Item = (&'a str, &'a C::Value)> {
        nested_map::entries_of(self.entities.components.store(), self.id)
    }
}

/// One entity's component `C`, to write. Every write is one `undo::Store::set`, with what it
/// reports.
pub struct Mut<'a, E: entity::Entities, C: Component> {
    writer: Writer<'a, E>,
    id: &'a E::Id,
    component: PhantomData<C>,
}

impl<'a, E: entity::Entities, C: Component> Mut<'a, E, C> {
    pub(in crate::state) fn new(writer: Writer<'a, E>, id: &'a E::Id) -> Self {
        Mut {
            writer,
            id,
            component: PhantomData,
        }
    }

    /// `UNKNOWN_ID` unless the entity exists: a value goes only on one that does.
    fn check_exists(&self) -> Result<(), Reason> {
        let exists = (self.writer.components).contains(self.writer.canon, self.id);
        exists.then_some(()).ok_or(Reason::UnknownId)
    }
}

/// A component that is there or not, such as a flag: every one is set the same way.
impl<E: entity::Has<C>, C: Component<Value = ()>> Mut<'_, E, C> {
    /// Put `name` on or take it off, reporting the change under `key`.
    fn set_with_key(&mut self, name: &str, on: bool, key: change::Key) -> Result<(), Reason> {
        self.check_exists()?;
        let change = Change::new(key, change::Value::Bool(!on), change::Value::Bool(on));
        let (id, value) = (self.id, on.then_some(()));
        (self.writer.components).set(self.writer.transaction, id, name, value, [change.into()])
    }
}

/// A component's text as a change value, `Empty` when there is none.
fn text_value(held: Option<impl AsRef<str>>) -> change::Value {
    held.map_or(change::Value::Empty, |held| {
        change::Value::Text(held.as_ref().to_string())
    })
}
