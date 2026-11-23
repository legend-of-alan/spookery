//! The scenario this property checks:
//!
//! ```gherkin
//! Feature: A hidden fact stays off a player's tape
//!
//!   Scenario: No random table leaks a hidden fact to a player
//!     Given the docks pack
//!     When places, known facts, hidden flags, and notes are shuffled
//!     Then no fact hidden from a player appears in that player's perceive output
//!     And it does not appear in the notes on that player's tape
//! ```
//!
//! The tape is the notes `perceive` shows that player. Deterministic in CI:
//! 64 cases, seed `0x0018_0018`, no failure file.

use std::path::PathBuf;

use crate::canon::Canon;
use crate::ecs::component::Note;
use crate::ecs::id::{ActorId, GlobalId, PlaceId};
use crate::ecs::{self, component, Presence};
use crate::rules::perceive::{self, View};
use crate::session;

use super::support::runner_config;
use proptest::prelude::*;
use proptest::test_runner::{TestCaseError, TestRunner};

#[derive(Clone, Debug)]
struct NotePlan {
    to_indices: Vec<usize>,
    place_index: Option<usize>,
    fact_ids: Vec<usize>,
}

#[derive(Clone, Debug)]
struct Scene {
    place_of: Vec<usize>,
    known: Vec<Vec<usize>>,
    hidden: Vec<bool>,
    notes: Vec<NotePlan>,
}

#[test]
fn no_hidden_fact_reaches_a_player() {
    let session = session::Session::open(&docks()).expect("docks pack opens");
    let actors: Vec<ActorId> = session
        .world()
        .actors()
        .query::<ActorId>()
        .iter()
        .cloned()
        .collect();
    let places: Vec<PlaceId> = session.canon().places.keys().cloned().collect();
    let facts: Vec<String> = session.canon().facts.keys().cloned().collect();
    assert!(
        !actors.is_empty() && !places.is_empty() && !facts.is_empty(),
        "the pack needs actors, places, and facts"
    );
    let canon = session.canon().clone();
    let world = session.world().clone();
    let mut runner = TestRunner::new(runner_config());
    runner
        .run(
            &pick_scene(actors.len(), places.len(), facts.len()),
            |scene| {
                let mut canon = canon.clone();
                let mut world = world.clone();
                apply_scene(&mut canon, &mut world, &actors, &places, &facts, &scene);
                assert_hidden_stays_hidden(&canon, &world)
            },
        )
        .unwrap_or_else(|err| panic!("visibility property failed: {err}"));
}

fn pick_scene(
    actor_count: usize,
    place_count: usize,
    fact_count: usize,
) -> impl Strategy<Value = Scene> {
    (
        prop::collection::vec(0..place_count, actor_count),
        prop::collection::vec(
            prop::collection::vec(0..fact_count, 0..=fact_count),
            actor_count,
        ),
        prop::collection::vec(any::<bool>(), fact_count),
        prop::collection::vec(pick_note(actor_count, place_count, fact_count), 0..=4),
    )
        .prop_map(|(place_of, known, hidden, notes)| Scene {
            place_of,
            known,
            hidden,
            notes,
        })
}

fn pick_note(
    actor_count: usize,
    place_count: usize,
    fact_count: usize,
) -> impl Strategy<Value = NotePlan> {
    (
        prop::collection::vec(0..actor_count, 0..=2),
        prop::option::of(0..place_count),
        prop::collection::vec(0..fact_count, 0..=2),
    )
        .prop_map(|(to_indices, place_index, fact_ids)| NotePlan {
            to_indices,
            place_index,
            fact_ids,
        })
}

fn apply_scene(
    canon: &mut Canon,
    world: &mut ecs::World,
    actors: &[ActorId],
    places: &[PlaceId],
    facts: &[String],
    scene: &Scene,
) {
    for (id, hidden) in facts.iter().zip(&scene.hidden) {
        if let Some(fact) = canon.facts.get_mut(id) {
            fact.hidden = *hidden;
        }
    }
    let mut save = serde_json::to_value(&**world).expect("the save serializes");
    save["actors"]["known_facts"] = serde_json::json!({});
    save["globals"]["notes"] = serde_json::json!({});
    **world = serde_json::from_value(save).expect("the save deserializes");
    for (actor_id, place_index) in actors.iter().zip(&scene.place_of) {
        let current = world.actors().get::<component::Place>(actor_id).get();
        if current.is_some() {
            let place = places[place_index % places.len()].as_str();
            world
                .actors_mut()
                .get_mut::<component::Place>(actor_id)
                .set(place)
                .unwrap();
        }
    }
    for (actor_id, known) in actors.iter().zip(&scene.known) {
        if !world.actors().contains(actor_id) {
            continue;
        }
        world
            .actors_mut()
            .get_mut::<component::KnownFacts>(actor_id)
            .learn(known.iter().map(|index| &facts[index % facts.len()]))
            .unwrap();
    }
    for (index, note) in scene.notes.iter().enumerate() {
        let note = Note {
            to_ids: (note.to_indices.iter())
                .map(|actor_index| actors[actor_index % actors.len()].clone())
                .collect(),
            place_id: note
                .place_index
                .map(|place_index| places[place_index % places.len()].clone()),
            fact_ids: (note.fact_ids.iter())
                .map(|fact| facts[fact % facts.len()].clone())
                .collect(),
            text: "The hour is late.".to_string(),
        };
        let mut globals = world.globals_mut();
        let id = format!("note-{index}");
        globals.get_mut::<Note>(&GlobalId).leave(&id, note).unwrap();
    }
}

fn assert_hidden_stays_hidden(canon: &Canon, world: &ecs::World) -> Result<(), TestCaseError> {
    let actors = world.actors().query::<(ActorId, &component::ControlType)>();
    for (pc, control_type) in actors.iter() {
        if !matches!(control_type, component::ControlType::Pc) {
            continue;
        }
        let known_facts = world.actors().get::<component::KnownFacts>(pc).get();
        let view = perceive::build_view(canon, world, &Presence::default(), pc)
            .map_err(|err| fail(err.to_string()))?;
        for (id, fact) in &canon.facts {
            if !fact.hidden || known_facts.is_some_and(|facts| facts.contains(id)) {
                continue;
            }
            if let Some(found) = shown(&view, id) {
                return Err(fail(format!("{pc} sees hidden {id} in {found}")));
            }
        }
    }
    Ok(())
}

/// Where a fact the player must not know showed up, if it did.
fn shown(view: &View, id: &str) -> Option<&'static str> {
    if view.known_facts.iter().any(|fact| fact.id == id) {
        return Some("perceive");
    }
    if view
        .unknown_mentions
        .iter()
        .any(|mentioned| mentioned == id)
    {
        return Some("perceive");
    }
    for note in &view.notes {
        if note.fact_ids.iter().any(|fact| fact == id) {
            return Some("tape");
        }
    }
    None
}

fn fail(message: String) -> TestCaseError {
    TestCaseError::fail(message)
}

fn docks() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/packs/docks")
}
