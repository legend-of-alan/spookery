//! The kernel crate has no cucumber harness. This is the scenario:
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

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use proptest::prelude::*;
use proptest::sample::select;
use proptest::test_runner::{Config, RngSeed, TestCaseError, TestRunner};
use spookery_kernel::apply::apply_effect;
use spookery_kernel::effect::{Effect, EffectClass};
use spookery_kernel::intent::{ActorKind, Intent, IntentArgs, Source, Target, ViewerId};
use spookery_kernel::model::Now;
use spookery_kernel::world::{Draft, World};

const CASES: u32 = 64;
const SEED: u64 = 0x0018_0018;

#[derive(Clone, Debug)]
enum Holder {
    Actor(String),
    Place(String),
}

#[derive(Clone, Debug)]
struct ItemChange {
    holder: Holder,
    id: String,
    delta: i64,
    max: Option<i64>,
}

struct TempPack(PathBuf);

impl Drop for TempPack {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn random_item_changes_never_leave_a_negative_count() {
    let pack = TempPack(copy_docks());
    let world = World::open(&pack.0).expect("docks pack opens");
    let actors: Vec<String> = world.now().actors.keys().cloned().collect();
    let places: Vec<String> = world.now().places.keys().cloned().collect();
    assert!(
        actors.len() >= 2 && !places.is_empty(),
        "the pack needs actors and places to shift"
    );
    let mut runner = TestRunner::new(config());
    let strategy = changes(&actors, &places);
    runner
        .run(&strategy, |steps| apply_changes(&world, &steps))
        .unwrap_or_else(|err| panic!("inventory property failed: {err}"));
}

fn config() -> Config {
    Config {
        cases: CASES,
        failure_persistence: None,
        rng_seed: RngSeed::Fixed(SEED),
        ..Config::default()
    }
}

fn apply_changes(world: &World, steps: &[ItemChange]) -> Result<(), TestCaseError> {
    let mut draft = Draft::begin(world);
    let intent = npc_intent();
    for step in steps {
        let before = draft.now().clone();
        let known = holder_known(draft.now(), &step.holder);
        let held = count_of(draft.now(), &step.holder, &step.id);
        let outcome = apply_effect(&mut draft, &intent, item_effect(step));
        no_stored_zero(draft.now())?;
        if !known {
            if outcome.is_ok() {
                return fail("an unknown holder accepted an item effect");
            }
            same_save(draft.now(), &before)?;
            continue;
        }
        let got = count_of(draft.now(), &step.holder, &step.id);
        match expected_count(held, step.delta, step.max) {
            None => {
                if outcome.is_ok() {
                    if takes_more_than_held(held, step.delta) {
                        return fail(format!(
                            "taking {} from {held} of {} was accepted and the count is {got}",
                            step.delta, step.id
                        ));
                    }
                    return fail(format!(
                        "an item effect on {held} of {} by {} was accepted",
                        step.id, step.delta
                    ));
                }
                same_save(draft.now(), &before)?;
                if takes_more_than_held(held, step.delta) && got != held {
                    return fail(format!(
                        "removing {} from {held} of {} changed the count to {got}",
                        step.delta, step.id
                    ));
                }
            }
            Some(expected) => {
                if outcome.is_err() {
                    return fail(format!(
                        "a legal item effect on {held} of {} by {} was refused",
                        step.id, step.delta
                    ));
                }
                if got != expected {
                    return fail(format!(
                        "count of {} became {got}, expected {expected}",
                        step.id
                    ));
                }
                if let Some(cap) = positive_cap(step.delta, step.max) {
                    if i64::from(got) > cap {
                        return fail(format!("count {got} is above max {cap}"));
                    }
                }
            }
        }
    }
    Ok(())
}

fn fail(message: impl Into<String>) -> Result<(), TestCaseError> {
    Err(TestCaseError::fail(message.into()))
}

fn same_save(now: &Now, before: &Now) -> Result<(), TestCaseError> {
    if now == before {
        Ok(())
    } else {
        fail("a refused item effect changed the save")
    }
}

/// A stored count is a `u32`, so it cannot be negative. A count of 0 is omitted.
fn no_stored_zero(now: &Now) -> Result<(), TestCaseError> {
    for (actor_id, actor) in &now.actors {
        for (item_id, count) in actor.inventory() {
            if *count == 0 {
                return fail(format!("{actor_id} stores a zero count of {item_id}"));
            }
        }
    }
    for (place_id, place) in &now.places {
        for (item_id, count) in &place.items {
            if *count == 0 {
                return fail(format!("{place_id} stores a zero count of {item_id}"));
            }
        }
    }
    Ok(())
}

fn holder_known(now: &Now, holder: &Holder) -> bool {
    match holder {
        Holder::Actor(id) => now.actors.contains_key(id),
        Holder::Place(id) => now.places.contains_key(id),
    }
}

fn count_of(now: &Now, holder: &Holder, id: &str) -> u32 {
    match holder {
        Holder::Actor(actor_id) => now
            .actors
            .get(actor_id)
            .map(|actor| actor.count(id))
            .unwrap_or(0),
        Holder::Place(place_id) => now
            .places
            .get(place_id)
            .and_then(|place| place.items.get(id))
            .copied()
            .unwrap_or(0),
    }
}

/// `max` caps a positive add. A result below 0, an overflowing add, a negative
/// cap on a positive add, or a result that does not fit in `u32` is refused.
fn expected_count(held: u32, delta: i64, max: Option<i64>) -> Option<u32> {
    let mut next = i64::from(held).checked_add(delta)?;
    if next < 0 {
        return None;
    }
    if delta > 0 {
        if let Some(cap) = max {
            if cap < 0 {
                return None;
            }
            if next > cap {
                next = cap;
            }
        }
    }
    u32::try_from(next).ok()
}

fn takes_more_than_held(held: u32, delta: i64) -> bool {
    delta < 0
        && i64::from(held)
            .checked_add(delta)
            .is_some_and(|next| next < 0)
}

fn positive_cap(delta: i64, max: Option<i64>) -> Option<i64> {
    match max {
        Some(cap) if delta > 0 && cap >= 0 => Some(cap),
        _ => None,
    }
}

fn npc_intent() -> Intent {
    Intent {
        id: "shift-item".to_string(),
        tick_index: 0,
        actor_id: "mira".to_string(),
        actor_kind: ActorKind::Npc,
        source: Source::Kernel,
        move_name: "shift_item".to_string(),
        target: Target::Empty,
        args: IntentArgs::default(),
        rationale_code: String::new(),
    }
}

fn item_effect(step: &ItemChange) -> Effect {
    let (actor_id, place) = match &step.holder {
        Holder::Actor(id) => (Some(id.clone()), None),
        Holder::Place(id) => (None, Some(id.clone())),
    };
    Effect {
        class: EffectClass::Item {
            actor_id,
            place,
            id: step.id.clone(),
            delta: step.delta,
            max: step.max,
        },
        as_viewer: ViewerId::Npc("mira".to_string()),
        intent_id: "shift-item".to_string(),
    }
}

/// Two actors, every place, and three item ids stay under `max_state_keys_mutated`.
/// A budget miss is the round's rollback, not a count change this property watches.
fn changes(actors: &[String], places: &[String]) -> impl Strategy<Value = Vec<ItemChange>> {
    prop::collection::vec(
        (
            holder_strategy(actors, places),
            item_id(),
            delta_value(),
            max_value(),
        )
            .prop_map(|(holder, id, delta, max)| ItemChange {
                holder,
                id,
                delta,
                max,
            }),
        1..=12,
    )
}

fn holder_strategy(actors: &[String], places: &[String]) -> impl Strategy<Value = Holder> {
    let mut known = Vec::new();
    for id in actors.iter().take(2) {
        known.push(Holder::Actor(id.clone()));
    }
    for id in places {
        known.push(Holder::Place(id.clone()));
    }
    prop_oneof![
        8 => select(known),
        1 => Just(Holder::Actor("nobody".to_string())),
        1 => Just(Holder::Place("nowhere".to_string())),
    ]
}

fn item_id() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("lamp_oil".to_string()),
        Just("coin".to_string()),
        Just("rope".to_string()),
    ]
}

fn delta_value() -> impl Strategy<Value = i64> {
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

fn max_value() -> impl Strategy<Value = Option<i64>> {
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

fn copy_docks() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let root = std::env::temp_dir().join(format!("spook-inventory-{}-{nanos}", std::process::id()));
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packs/oneshot-docks");
    copy_dir(&source, &root);
    root
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("temp pack directory");
    for entry in fs::read_dir(from).expect("pack directory") {
        let entry = entry.expect("pack entry");
        let dest = to.join(entry.file_name());
        if entry.file_type().expect("pack entry type").is_dir() {
            copy_dir(&entry.path(), &dest);
        } else {
            fs::copy(entry.path(), dest).expect("copy pack file");
        }
    }
}
