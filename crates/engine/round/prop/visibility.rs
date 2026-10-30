//! The kernel crate has no cucumber harness. This is the scenario:
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
use crate::model::{sort_unique, Note, Presence, State};
use crate::perceive::{perceive_as, View};
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
    let actors: Vec<String> = world.state().actors.keys().cloned().collect();
    let places: Vec<String> = world.state().places.keys().cloned().collect();
    let facts: Vec<String> = world.canon().facts.keys().cloned().collect();
    assert!(
        !actors.is_empty() && !places.is_empty() && !facts.is_empty(),
        "the pack needs actors, places, and facts"
    );
    let canon = world.canon().clone();
    let now = world.state().clone();
    let mut runner = TestRunner::new(runner_config());
    runner
        .run(
            &pick_scene(actors.len(), places.len(), facts.len()),
            |scene| {
                let mut canon = canon.clone();
                let mut now = now.clone();
                apply_scene(&mut canon, &mut now, &actors, &places, &facts, &scene);
                assert_hidden_stays_hidden(&canon, &now)
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
    now: &mut State,
    actors: &[String],
    places: &[String],
    facts: &[String],
    scene: &Scene,
) {
    for (id, hidden) in facts.iter().zip(&scene.hidden) {
        if let Some(fact) = canon.facts.get_mut(id) {
            fact.hidden = *hidden;
        }
    }
    for (actor_id, place_index) in actors.iter().zip(&scene.place_of) {
        move_actor(now, actor_id, &places[place_index % places.len()]);
    }
    for (actor_id, known) in actors.iter().zip(&scene.known) {
        let Some(actor) = now.actors.get_mut(actor_id) else {
            continue;
        };
        *actor.known_facts_mut() = known
            .iter()
            .map(|index| facts[index % facts.len()].clone())
            .collect();
        sort_unique(actor.known_facts_mut());
    }
    now.notes.clear();
    for (index, note) in scene.notes.iter().enumerate() {
        let mut fact_ids: Vec<String> = note
            .fact_ids
            .iter()
            .map(|fact| facts[fact % facts.len()].clone())
            .collect();
        sort_unique(&mut fact_ids);
        let mut to: Vec<String> = note
            .to
            .iter()
            .map(|actor| actors[actor % actors.len()].clone())
            .collect();
        sort_unique(&mut to);
        now.notes.push(Note {
            id: format!("note-{index}"),
            to,
            place: note.place.map(|place| places[place % places.len()].clone()),
            fact_ids,
            text: "The hour is late.".to_string(),
        });
    }
}

fn move_actor(now: &mut State, actor_id: &str, place: &str) {
    let old = now
        .actors
        .get(actor_id)
        .map(|actor| actor.place().to_string());
    if let Some(old) = old {
        if let Some(live) = now.places.get_mut(&old) {
            live.actors.retain(|id| id != actor_id);
        }
    }
    if let Some(actor) = now.actors.get_mut(actor_id) {
        *actor.place_mut() = place.to_string();
    }
    if let Some(live) = now.places.get_mut(place) {
        if !live.actors.iter().any(|id| id == actor_id) {
            live.actors.push(actor_id.to_string());
            live.actors.sort();
        }
    }
}

fn assert_hidden_stays_hidden(canon: &Canon, now: &State) -> Result<(), TestCaseError> {
    for (pc, actor) in &now.actors {
        if !actor.is_pc() {
            continue;
        }
        let view = perceive_as(
            canon,
            now,
            &Presence::default(),
            &Viewer::Player(pc.clone()),
        )
        .map_err(|err| fail(err.to_string()))?;
        for (id, fact) in &canon.facts {
            if !fact.hidden || actor.known_facts().iter().any(|known| known == id) {
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
