//! Reads that walk many entities of one collection, shaped like Bevy's `Query`: a query reads
//! `D` from each entity that passes the filter `F`. It only reads; writes go through a `Writer`.

use std::marker::PhantomData;

use crate::state::component;
use crate::state::entity::{self, Actors, Globals, Places, Stories};
use crate::state::id::{ActorId, GlobalId, PlaceId, StoryId};
use crate::state::Reader;

/// Entity ids, in id order, as a query walks them.
pub(in crate::state) type Ids<'a, E> =
    Box<dyn Iterator<Item = &'a <E as entity::Entities>::Id> + 'a>;

/// What a query reads from each entity: its id, a component, `Option` of one, or a tuple of up to
/// four of them. `&C` gives the value of a component an entity holds one of, and a
/// `component::Ref` of one it holds many of, by name.
pub trait QueryData<E: entity::Entities> {
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
pub trait QueryFilter<E: entity::Entities> {
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
pub struct Query<'a, E, D, F = ()> {
    pub(in crate::state) entities: Reader<'a, E>,
    pub(in crate::state) data_and_filter: PhantomData<fn() -> (D, F)>,
}

impl<'a, E: entity::Entities, D: QueryData<E>, F: QueryFilter<E>> Query<'a, E, D, F> {
    /// What each entity that passes gives, in id order. The items borrow the save, not the query.
    pub fn iter(&self) -> impl Iterator<Item = D::Item<'a>> {
        let entities = self.entities;
        let carriers = D::carriers(entities).or_else(|| F::carriers(entities));
        let ids = carriers.unwrap_or_else(|| entities.components.ids(entities.canon));
        let passing = ids.filter(move |id| F::matches(entities, id));
        passing.filter_map(move |id| D::fetch(entities, id))
    }

    /// What entity `id` gives, if it passes. No caller uses it yet.
    pub fn get(&self, id: &'a E::Id) -> Option<D::Item<'a>> {
        let passes = F::matches(self.entities, id);
        passes.then(|| D::fetch(self.entities, id)).flatten()
    }

    /// Whether entity `id` passes and gives what `D` reads. No caller uses it yet.
    pub fn contains(&self, id: &'a E::Id) -> bool {
        self.get(id).is_some()
    }
}

/// Keep the entities that hold `C`.
pub struct With<C>(PhantomData<C>);

/// Keep the entities that do not hold `C`.
pub struct Without<C>(PhantomData<C>);

impl<E: entity::Entities, C: 'static> QueryFilter<E> for With<C>
where
    for<'x> &'x C: QueryData<E>,
{
    fn carriers<'a>(entities: Reader<'a, E>) -> Option<Ids<'a, E>> {
        <&C>::carriers(entities)
    }

    fn matches<'a>(entities: Reader<'a, E>, id: &'a E::Id) -> bool {
        <&C>::fetch(entities, id).is_some()
    }
}

impl<E: entity::Entities, C: 'static> QueryFilter<E> for Without<C>
where
    for<'x> &'x C: QueryData<E>,
{
    fn matches<'a>(entities: Reader<'a, E>, id: &'a E::Id) -> bool {
        <&C>::fetch(entities, id).is_none()
    }
}

impl<E: entity::Entities> QueryFilter<E> for () {
    fn matches<'a>(_: Reader<'a, E>, _: &'a E::Id) -> bool {
        true
    }
}

/// A component an entity holds many of gives its `component::Ref`, to read by name.
impl<E: entity::Has<C>, C: component::Component> QueryData<E> for &C {
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
impl<'c, E: entity::Entities, C> QueryData<E> for Option<&'c C>
where
    &'c C: QueryData<E>,
{
    type Item<'a>
        = Option<<&'c C as QueryData<E>>::Item<'a>>
    where
        E: 'a;

    fn fetch<'a>(entities: Reader<'a, E>, id: &'a E::Id) -> Option<Self::Item<'a>> {
        Some(<&'c C>::fetch(entities, id))
    }
}

/// Declares that `$entities` holds at most one `$component` per entity, in its field `$store`.
macro_rules! has_one {
    ($entities:ty, $component:ty, $store:ident) => {
        const _: () = {
            use $crate::state::entity::{self, query::Ids, QueryData};
            use $crate::state::{component, Reader};

            impl QueryData<$entities> for &$component {
                type Item<'a> = &'a <$component as component::Component>::Value;

                fn carriers<'a>(entities: Reader<'a, $entities>) -> Option<Ids<'a, $entities>> {
                    Some(Box::new(entities.components.$store.keys()))
                }

                fn fetch<'a>(
                    entities: Reader<'a, $entities>,
                    id: &'a <$entities as entity::Entities>::Id,
                ) -> Option<Self::Item<'a>> {
                    entities.components.$store.get(id)
                }
            }
        };
    };
}
pub(in crate::state) use has_one;

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
        impl<E: entity::Entities, $($part: QueryData<E>),+> QueryData<E> for ($($part,)+) {
            type Item<'a> = ($($part::Item<'a>,)+) where E: 'a;

            fn carriers<'a>(entities: Reader<'a, E>) -> Option<Ids<'a, E>> {
                None $(.or_else(|| $part::carriers(entities)))+
            }

            fn fetch<'a>(entities: Reader<'a, E>, id: &'a E::Id) -> Option<Self::Item<'a>> {
                Some(($($part::fetch(entities, id)?,)+))
            }
        }

        impl<E: entity::Entities, $($part: QueryFilter<E>),+> QueryFilter<E> for ($($part,)+) {
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

#[cfg(test)]
mod tests {
    use crate::state::component::{self, Meter};
    use crate::state::entity::{With, Without};
    use crate::state::id::{ActorId, PlaceId};
    use crate::World;

    fn docks() -> World<()> {
        let pack =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packs/oneshot-docks");
        World::open(&pack).expect("open")
    }

    #[test]
    fn a_query_reads_each_part_of_a_tuple_from_every_entity_that_holds_them_all() {
        let world = docks();
        let actors = world.state().actors();
        let query = actors.query::<(ActorId, &component::ControlType, &component::Place, &Meter)>();
        let read: Vec<_> = (query.iter())
            .map(|(id, control_type, place, meters)| {
                (
                    id.as_str(),
                    control_type.is_pc(),
                    place.as_str(),
                    meters.iter().count(),
                )
            })
            .collect();
        assert_eq!(read[0], ("aldric", false, "docks_shop", 2));
        assert_eq!(read[4], ("rowan", true, "docks_shop", 3));
        assert_eq!(read.len(), 6, "every actor holds all four");
        let knowing = actors.query::<(ActorId, &component::KnownFacts)>();
        let knowing: Vec<_> = knowing
            .iter()
            .map(|(id, facts)| (id.as_str(), facts.len()))
            .collect();
        assert_eq!(knowing, [("mira", 1)], "only Mira knows a fact");
        let places = world.state().places().query::<PlaceId>();
        assert_eq!(
            places.iter().count(),
            world.canon().places.len(),
            "every place the pack defines"
        );
    }

    #[test]
    fn with_and_without_keep_the_entities_that_hold_or_lack_a_component() {
        let world = docks();
        let actors = world.state().actors();
        let knowing = actors.query_filtered::<ActorId, With<component::KnownFacts>>();
        assert_eq!(knowing.iter().collect::<Vec<_>>(), ["mira"]);
        let filter = actors
            .query_filtered::<ActorId, (With<component::Visit>, Without<component::KnownFacts>)>();
        assert_eq!(
            filter.iter().collect::<Vec<_>>(),
            ["harbormouse", "rowan", "wren"]
        );
        let with_oil = world
            .state()
            .places()
            .query_filtered::<PlaceId, With<component::Item>>();
        assert_eq!(with_oil.iter().collect::<Vec<_>>(), ["docks_shop"]);
    }

    #[test]
    fn an_option_walks_every_entity_and_get_and_contains_read_one() {
        let world = docks();
        let actors = world.state().actors();
        let facts = actors.query::<Option<&component::KnownFacts>>();
        let counts: Vec<_> = facts.iter().map(|facts| facts.map(Vec::len)).collect();
        assert_eq!(
            counts,
            [None, None, None, Some(1), None, None],
            "Mira knows one"
        );
        let ids = actors.query::<(ActorId, Option<&component::KnownFacts>)>();
        let knowing = ids.iter().filter(|(_, facts)| facts.is_some());
        assert_eq!(knowing.map(|(id, _)| id).collect::<Vec<_>>(), ["mira"]);
        let (mira, rowan) = (ActorId::from("mira"), ActorId::from("rowan"));
        let knowing = actors.query::<&component::KnownFacts>();
        assert_eq!(knowing.get(&mira).map(Vec::len), Some(1));
        assert_eq!(knowing.get(&rowan), None);
        assert!(knowing.contains(&mira) && !knowing.contains(&rowan));
        let unknowing = actors.query_filtered::<ActorId, Without<component::KnownFacts>>();
        assert!(!unknowing.contains(&mira) && unknowing.contains(&rowan));
    }
}
