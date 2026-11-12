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
use crate::intent::Viewer;
use crate::rules::perceive::{perceive_as, View};
use crate::state::id::{ActorId, PlaceId};
use crate::state::{component, sort_unique, ActorKind, Note, Presence, State};
use crate::world::World;

use super::support::runner_config;
use proptest::prelude::*;
use proptest::test_runner::{TestCaseError, TestRunner};

#[derive(Clone, Debug)]
struct NotePlan {
    to: Vec<usize>,
    place: Option<usize>,
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
    let world = World::open(&docks()).expect("docks pack opens");
    let actors: Vec<ActorId> = world.state().actors().kind.keys().cloned().collect();
    let places: Vec<PlaceId> = world.canon().places.keys().cloned().collect();
    let facts: Vec<String> = world.canon().facts.keys().cloned().collect();
    assert!(
        !actors.is_empty() && !places.is_empty() && !facts.is_empty(),
        "the pack needs actors, places, and facts"
    );
    let canon = world.canon().clone();
    let state = world.state().clone();
    let mut runner = TestRunner::new(runner_config());
    runner
        .run(
            &pick_scene(actors.len(), places.len(), facts.len()),
            |scene| {
                let mut canon = canon.clone();
                let mut state = state.clone();
                apply_scene(&mut canon, &mut state, &actors, &places, &facts, &scene);
                assert_hidden_stays_hidden(&canon, &state)
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
        .prop_map(|(to, place, fact_ids)| NotePlan {
            to,
            place,
            fact_ids,
        })
}

fn apply_scene(
    canon: &mut Canon,
    state: &mut State,
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
    // No writer forgets a fact or takes a note back, so the scene starts from the save as the
    // file holds it with neither.
    let mut save = serde_json::to_value(&**state).expect("the save serializes");
    save["actors"]["known_facts"] = serde_json::json!({});
    save["notes"] = serde_json::json!([]);
    **state = serde_json::from_value(save).expect("the save deserializes");
    for (actor_id, place_index) in actors.iter().zip(&scene.place_of) {
        if state.actors().place.contains_key(actor_id) {
            let place = places[place_index % places.len()].as_str();
            state
                .actors_mut()
                .get_mut::<component::Place>(actor_id)
                .set(place)
                .unwrap();
        }
    }
    for (actor_id, known) in actors.iter().zip(&scene.known) {
        if !state.actors().kind.contains_key(actor_id) {
            continue;
        }
        let mut known_facts: Vec<String> = known
            .iter()
            .map(|index| facts[index % facts.len()].clone())
            .collect();
        sort_unique(&mut known_facts);
        state
            .actors_mut()
            .get_mut::<component::KnownFacts>(actor_id)
            .learn(&known_facts)
            .unwrap();
    }
    for (index, note) in scene.notes.iter().enumerate() {
        let mut fact_ids: Vec<String> = note
            .fact_ids
            .iter()
            .map(|fact| facts[fact % facts.len()].clone())
            .collect();
        sort_unique(&mut fact_ids);
        let mut to: Vec<ActorId> = note
            .to
            .iter()
            .map(|actor| actors[actor % actors.len()].clone())
            .collect();
        sort_unique(&mut to);
        state
            .notes_mut()
            .leave(Note {
                id: format!("note-{index}"),
                to,
                place: note.place.map(|place| places[place % places.len()].clone()),
                fact_ids,
                text: "The hour is late.".to_string(),
            })
            .unwrap();
    }
}

fn assert_hidden_stays_hidden(canon: &Canon, state: &State) -> Result<(), TestCaseError> {
    for (pc, kind) in state.actors().kind.iter() {
        if *kind != ActorKind::Pc {
            continue;
        }
        let known_facts = state.actors().get::<component::KnownFacts>(pc).get();
        let view = perceive_as(
            canon,
            state,
            &Presence::default(),
            &Viewer::Player(pc.clone()),
        )
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
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packs/oneshot-docks")
}
