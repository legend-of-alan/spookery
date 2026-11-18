//! Reads that walk many entities of one collection, shaped like Bevy's `Query`: a query reads
//! `D` from each entity that passes the filter `F`. It only reads; writes go through a `Writer`.

use std::marker::PhantomData;

use crate::ecs::component;
use crate::ecs::entity::{self, Actors, Globals, Places, Stories};
use crate::ecs::id::{ActorId, GlobalId, PlaceId, StoryId};
use crate::ecs::Reader;

/// Entity ids, in id order, as a query walks them.
pub(in crate::ecs) type Ids<'a, E> =
    Box<dyn Iterator<Item = &'a <E as entity::Collection>::Id> + 'a>;

/// What a query reads from each entity: its id, a component, `Option` of one, or a tuple of up to
/// four of them. `&C` gives the value of a component an entity holds one of, and a
/// `component::Ref` of one it holds many of, by entry id.
pub trait QueryData<E: entity::Collection> {
    // repeats module: mirrors Bevy's QueryData (Naming rule 15)
    type Item<'a>
    where
        E: 'a;

    /// The entities that carry the first component this reads, or `None` when it reads none.
    fn carriers<'a>(_: Reader<'a, E>) -> Option<Ids<'a, E>> {
        None
    }

    /// What entity `id` gives, or `None` when it lacks a component this reads.
    fn fetch<'a>(entities: Reader<'a, E>, id: &'a E::Id) -> Option<Self::Item<'a>>;
}

/// Which entities a query keeps: `With<C>`, `Without<C>`, or a tuple of up to four of them.
pub trait QueryFilter<E: entity::Collection> {
    // repeats module: mirrors Bevy's QueryFilter (Naming rule 15)
    /// The entities that carry the first component this requires, or `None` when it requires none.
    fn carriers<'a>(_: Reader<'a, E>) -> Option<Ids<'a, E>> {
        None
    }

    /// Whether entity `id` passes.
    fn matches<'a>(entities: Reader<'a, E>, id: &'a E::Id) -> bool;
}

/// The entities of one collection that pass `F`, and what `D` reads from each. It walks the
/// carriers of the first component `D` reads, else of the first `F` requires, else every entity
/// the collection lists; a component the save holds for an entity that does not exist is walked
/// too.
pub struct Query<'w, E, D, F = ()> {
    pub(in crate::ecs) entities: Reader<'w, E>,
    pub(in crate::ecs) data_and_filter: PhantomData<fn() -> (D, F)>,
}

impl<'w, E: entity::Collection, D: QueryData<E>, F: QueryFilter<E>> Query<'w, E, D, F> {
    /// What each entity that passes gives, in id order. The items borrow the save, not the query.
    pub fn iter(&self) -> impl Iterator<Item = D::Item<'w>> {
        let entities = self.entities;
        let carriers = D::carriers(entities).or_else(|| F::carriers(entities));
        let ids = carriers.unwrap_or_else(|| entities.components.ids(entities.canon));
        let passing = ids.filter(move |id| F::matches(entities, id));
        passing.filter_map(move |id| D::fetch(entities, id))
    }

    /// What entity `id` gives, if it passes. No caller uses it yet.
    pub fn get(&self, id: &'w E::Id) -> Option<D::Item<'w>> {
        let passes = F::matches(self.entities, id);
        passes.then(|| D::fetch(self.entities, id)).flatten()
    }

    /// Whether entity `id` passes and gives what `D` reads. No caller uses it yet.
    pub fn contains(&self, id: &'w E::Id) -> bool {
        self.get(id).is_some()
    }
}

/// Keep the entities that hold `C`.
pub struct With<C>(PhantomData<C>);

/// Keep the entities that do not hold `C`.
pub struct Without<C>(PhantomData<C>);

impl<E: entity::Collection, C: 'static> QueryFilter<E> for With<C>
where
    for<'a> &'a C: QueryData<E>,
{
    fn carriers<'a>(entities: Reader<'a, E>) -> Option<Ids<'a, E>> {
        <&C>::carriers(entities)
    }

    fn matches<'a>(entities: Reader<'a, E>, id: &'a E::Id) -> bool {
        <&C>::fetch(entities, id).is_some()
    }
}

impl<E: entity::Collection, C: 'static> QueryFilter<E> for Without<C>
where
    for<'a> &'a C: QueryData<E>,
{
    fn matches<'a>(entities: Reader<'a, E>, id: &'a E::Id) -> bool {
        <&C>::fetch(entities, id).is_none()
    }
}

impl<E: entity::Collection> QueryFilter<E> for () {
    fn matches<'a>(_: Reader<'a, E>, _: &'a E::Id) -> bool {
        true
    }
}

/// A component an entity holds many of gives its `component::Ref`, to read by entry id.
impl<E, C> QueryData<E> for &C
where
    E: entity::Has<C, Store = entity::MapStore<E, C>>,
    C: component::Component,
{
    type Item<'a>
        = component::Ref<'a, E, C>
    where
        E: 'a;

    fn carriers<'a>(entities: Reader<'a, E>) -> Option<Ids<'a, E>> {
        let mut last = None;
        let ids = (entities.components.store().keys()).map(|(id, _)| id);
        Some(Box::new(
            ids.filter(move |id| last.replace(*id) != Some(*id)),
        ))
    }

    fn fetch<'a>(entities: Reader<'a, E>, id: &'a E::Id) -> Option<Self::Item<'a>> {
        let held = entities.get::<C>(id);
        held.iter().next().is_some().then_some(held)
    }
}

/// `Option<&C>` gives what `&C` gives, or `None` for an entity without `C`. It names no carriers,
/// so it never picks which entities a query walks: alone, it walks every entity.
impl<'a, E: entity::Collection, C> QueryData<E> for Option<&'a C>
where
    &'a C: QueryData<E>,
{
    type Item<'w>
        = Option<<&'a C as QueryData<E>>::Item<'w>>
    where
        E: 'w;

    fn fetch<'w>(entities: Reader<'w, E>, id: &'w E::Id) -> Option<Self::Item<'w>> {
        Some(<&'a C>::fetch(entities, id))
    }
}

/// The entity's own id, in the place of Bevy's `Entity`.
macro_rules! id_data {
    ($($entities:ty: $id:ty),+) => {$(
        impl QueryData<$entities> for $id {
            type Item<'a> = &'a $id;

            fn fetch<'a>(_: Reader<'a, $entities>, id: &'a $id) -> Option<&'a $id> {
                Some(id)
            }
        }
    )+};
}
id_data!(Actors: ActorId, Places: PlaceId, Stories: StoryId, Globals: GlobalId);

/// Tuples of up to four parts read each part, and keep an entity when every part does.
macro_rules! tuples {
    ($($part:ident),+) => {
        impl<E: entity::Collection, $($part: QueryData<E>),+> QueryData<E> for ($($part,)+) {
            type Item<'a> = ($($part::Item<'a>,)+) where E: 'a;

            fn carriers<'a>(entities: Reader<'a, E>) -> Option<Ids<'a, E>> {
                None $(.or_else(|| $part::carriers(entities)))+
            }

            fn fetch<'a>(entities: Reader<'a, E>, id: &'a E::Id) -> Option<Self::Item<'a>> {
                Some(($($part::fetch(entities, id)?,)+))
            }
        }

        impl<E: entity::Collection, $($part: QueryFilter<E>),+> QueryFilter<E> for ($($part,)+) {
            fn carriers<'a>(entities: Reader<'a, E>) -> Option<Ids<'a, E>> {
                None $(.or_else(|| $part::carriers(entities)))+
            }

            fn matches<'a>(entities: Reader<'a, E>, id: &'a E::Id) -> bool {
                true $(&& $part::matches(entities, id))+
            }
        }
    };
}
tuples!(A, B);
tuples!(A, B, C);
tuples!(A, B, C, D);
