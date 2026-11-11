//! The scenario this property checks:
//!
//! ```gherkin
//! Feature: Inventory never goes negative
//!
//!   Scenario: Random item changes never leave a negative count
//!     Given the docks pack
//!     When random item effects are applied to an actor or a place
//!     Then no count is negative
//!     And a refused effect leaves the save unchanged
//!     And a positive add never leaves a count above that effect's max
//!     And taking away more than is held is refused
//! ```
//!
//! Deterministic in CI: 64 cases, seed `0x0018_0018`, no failure file.

use crate::canon::Canon;
use crate::entity;
use crate::intent::{Intent, IntentArgs, Source, Target, Viewer};
use crate::rules::effect::{Effect, EffectKind};
use crate::state::id::{ActorId, PlaceId};
use crate::state::State;
use crate::testing::copy_docks;
use crate::world::World;

use super::support::{runner_config, TempPack};
use proptest::prelude::*;
use proptest::sample::select;
use proptest::test_runner::{TestCaseError, TestRunner};

#[derive(Clone, Debug)]
struct ItemChange {
    carrier: entity::Id,
    id: String,
    delta: i64,
    max: Option<i64>,
}

#[test]
fn random_item_changes_never_leave_a_negative_count() {
    let pack = TempPack(copy_docks("inventory"));
    let world = World::open(&pack.0).expect("docks pack opens");
    let actors: Vec<ActorId> = world.state().actors().kind.keys().cloned().collect();
    let places: Vec<PlaceId> = world.canon().places.keys().cloned().collect();
    assert!(
        actors.len() >= 2 && !places.is_empty(),
        "the pack needs actors and places to shift"
    );
    let mut runner = TestRunner::new(runner_config());
    let strategy = pick_changes(&actors, &places);
    runner
        .run(&strategy, |steps| apply_changes(&world, &steps))
        .unwrap_or_else(|err| panic!("inventory property failed: {err}"));
}

fn apply_changes(world: &World<()>, steps: &[ItemChange]) -> Result<(), TestCaseError> {
    let nobody = crate::state::Presence::default();
    let (canon, mut state) = (world.canon(), world.begin_round());
    let intent = npc_intent();
    for step in steps {
        let before = state.clone();
        let known = is_known_entity(canon, &state, &step.carrier);
        let held = count_of(&state, &step.carrier, &step.id);
        let outcome = item_effect(step).apply(canon, &mut state, &nobody, &intent);
        assert_no_stored_zero(&state)?;
        if !known {
            if outcome.is_ok() {
                return fail("an unknown entity accepted an item effect");
            }
            assert_same_save(&state, &before)?;
            continue;
        }
        if outcome.is_err() {
            assert_same_save(&state, &before)?;
            continue;
        }
        let got = count_of(&state, &step.carrier, &step.id);
        if takes_more_than_held(held, step.delta) {
            return fail(format!(
                "taking {} from {held} of {} was accepted and the count is {got}",
                step.delta, step.id
            ));
        }
        // A negative max is not a ceiling. Only a positive add is capped.
        if step.delta > 0 {
            if let Some(cap) = step.max.filter(|cap| *cap >= 0) {
                if i64::from(got) > cap {
                    return fail(format!("count {got} is above max {cap}"));
                }
            }
        }
    }
    Ok(())
}

fn fail(message: impl Into<String>) -> Result<(), TestCaseError> {
    Err(TestCaseError::fail(message.into()))
}

fn assert_same_save(state: &State, before: &State) -> Result<(), TestCaseError> {
    if state == before {
        Ok(())
    } else {
        fail("a refused item effect changed the save")
    }
}

/// A stored count is a `u32`, so it cannot be negative. A count of 0 is omitted.
fn assert_no_stored_zero(state: &State) -> Result<(), TestCaseError> {
    for ((actor_id, item_id), count) in state.actors().items.iter() {
        if *count == 0 {
            return fail(format!("{actor_id} stores a zero count of {item_id}"));
        }
    }
    for ((place_id, item_id), count) in state.places().items.iter() {
        if *count == 0 {
            return fail(format!("{place_id} stores a zero count of {item_id}"));
        }
    }
    Ok(())
}

fn is_known_entity(canon: &Canon, state: &State, carrier: &entity::Id) -> bool {
    match carrier {
        entity::Id::Actor(id) => state.actors().kind.contains_key(id),
        entity::Id::Place(id) => canon.places.contains_key(id),
        entity::Id::Story(_) | entity::Id::Global => false,
    }
}

fn count_of(state: &State, carrier: &entity::Id, id: &str) -> u32 {
    match carrier {
        entity::Id::Actor(actor_id) => state
            .actors()
            .items
            .get(&(actor_id.clone(), id.to_string()))
            .copied()
            .unwrap_or(0),
        entity::Id::Place(place_id) => state
            .places()
            .items
            .get(&(place_id.clone(), id.to_string()))
            .copied()
            .unwrap_or(0),
        entity::Id::Story(_) | entity::Id::Global => 0,
    }
}

fn takes_more_than_held(held: u32, delta: i64) -> bool {
    delta < 0
        && i64::from(held)
            .checked_add(delta)
            .is_some_and(|next| next < 0)
}

fn npc_intent() -> Intent {
    Intent {
        id: "shift-item".to_string(),
        round_index: 0,
        actor: Some(Viewer::Npc("mira".into())),
        source: Source::Engine,
        move_name: "shift_item".to_string(),
        target: Target::Empty,
        args: IntentArgs::default(),
        rationale_code: String::new(),
    }
}

fn item_effect(step: &ItemChange) -> Effect {
    let (actor_id, place) = match &step.carrier {
        entity::Id::Actor(id) => (Some(id.clone()), None),
        entity::Id::Place(id) => (None, Some(id.clone())),
        entity::Id::Story(_) | entity::Id::Global => (None, None),
    };
    Effect {
        kind: EffectKind::Item {
            actor_id,
            place,
            id: step.id.clone(),
            delta: step.delta,
            max: step.max,
        },
        viewer: Some(Viewer::Npc("mira".into())),
        intent_id: "shift-item".to_string(),
    }
}

/// Two actors, every place, and three item ids stay under `max_state_keys_mutated`.
/// A budget miss is the round's rollback, not a count change this property watches.
fn pick_changes(actors: &[ActorId], places: &[PlaceId]) -> impl Strategy<Value = Vec<ItemChange>> {
    prop::collection::vec(
        (
            pick_entity(actors, places),
            pick_item_id(),
            pick_delta(),
            pick_max(),
        )
            .prop_map(|(carrier, id, delta, max)| ItemChange {
                carrier,
                id,
                delta,
                max,
            }),
        1..=12,
    )
}

fn pick_entity(actors: &[ActorId], places: &[PlaceId]) -> impl Strategy<Value = entity::Id> {
    let mut known = Vec::new();
    for id in actors.iter().take(2) {
        known.push(entity::Id::Actor(id.clone()));
    }
    for id in places {
        known.push(entity::Id::Place(id.clone()));
    }
    prop_oneof![
        8 => select(known),
        1 => Just(entity::Id::Actor("nobody".into())),
        1 => Just(entity::Id::Place("nowhere".into())),
    ]
}

fn pick_item_id() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("lamp_oil".to_string()),
        Just("coin".to_string()),
        Just("rope".to_string()),
    ]
}

fn pick_delta() -> impl Strategy<Value = i64> {
    prop_oneof![
        5 => -8i64..=8,
        2 => -10_000i64..=10_000,
        2 => any::<i64>(),
        1 => select(vec![
            i64::MIN,
            i64::MIN + 1,
            i64::MAX,
            i64::MAX - 1,
            0,
            1,
            -1,
            i64::from(u32::MAX),
            i64::from(u32::MAX) + 1,
        ]),
    ]
}

fn pick_max() -> impl Strategy<Value = Option<i64>> {
    prop_oneof![
        4 => Just(None),
        3 => (0i64..=32).prop_map(Some),
        1 => select(vec![
            -1_i64,
            i64::MIN,
            0,
            i64::from(u32::MAX),
            i64::from(u32::MAX) + 1,
            i64::MAX,
        ])
        .prop_map(Some),
        2 => any::<i64>().prop_map(Some),
    ]
}
