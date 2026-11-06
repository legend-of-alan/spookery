//! The scenario this property checks:
//!
//! ```gherkin
//! Feature: A rollback puts the save back exactly
//!
//!   Scenario: Random writes under nested savepoints roll back to each savepoint's save
//!     Given the docks pack, with no live record yet for the street
//!     When random effects land under savepoints opened and rolled back at random
//!     Then each rollback leaves the save as it was when its savepoint was taken
//!     And rolling back to the first savepoint leaves no change to report
//! ```
//!
//! Deterministic in CI: 64 cases, seed `0x0018_0018`, no failure file.

use std::sync::Arc;

use crate::draft::Draft;
use crate::intent::{Intent, IntentArgs, Source, Target, Viewer};
use crate::model::meter::MeterWrite;
use crate::model::State;
use crate::rules::effect::{Effect, EffectKind, Holder};
use crate::testing::copy_docks;
use crate::world::World;

use super::support::{runner_config, TempPack};
use proptest::prelude::*;
use proptest::sample::select;
use proptest::test_runner::{TestCaseError, TestRunner};

#[derive(Clone, Debug)]
enum Step {
    Savepoint,
    Rollback,
    Item(Holder, &'static str, i64),
    Meter(&'static str, i64),
    Door(bool),
    WorldFlag(bool),
    Walk(&'static str),
    Note,
    Reveal(&'static str),
}

#[test]
fn nested_rollbacks_put_the_save_back_exactly() {
    let pack = TempPack(copy_docks("rollback"));
    let opened = World::open(&pack.0).expect("docks pack opens");
    // The street has no live record yet, so walking there gives it its first one.
    let mut state = opened.state().clone();
    state.places.remove("docks_street");
    let world = World::from_parts(&pack.0, Arc::new(opened.canon().clone()), state);
    let mut runner = TestRunner::new(runner_config());
    runner
        .run(&prop::collection::vec(pick_step(), 1..40), |steps| {
            play_steps(&world, &steps)
        })
        .unwrap_or_else(|err| panic!("rollback property failed: {err}"));
}

fn play_steps(world: &World<()>, steps: &[Step]) -> Result<(), TestCaseError> {
    let nobody = crate::model::Presence::default();
    let mut draft = Draft::begin(world, &nobody);
    let intent = mira_intent();
    let mut open = vec![(draft.savepoint(), draft.state().clone())];
    for (index, step) in steps.iter().enumerate() {
        match step {
            Step::Savepoint => open.push((draft.savepoint(), draft.state().clone())),
            Step::Rollback if open.len() > 1 => {
                let (point, saved) = open.pop().expect("an open savepoint");
                draft.rollback(point);
                same_save(draft.state(), &saved)?;
            }
            Step::Rollback => {}
            _ => {
                // A refused write is left for a savepoint to undo, as in a round.
                let _ = effect(&draft, step, index).apply(&mut draft, &intent);
            }
        }
    }
    while let Some((point, saved)) = open.pop() {
        draft.rollback(point);
        same_save(draft.state(), &saved)?;
    }
    if !draft.take_changes().is_empty() {
        return Err(TestCaseError::fail("a rolled-back write is still reported"));
    }
    Ok(())
}

fn same_save(state: &State, saved: &State) -> Result<(), TestCaseError> {
    if state == saved {
        Ok(())
    } else {
        Err(TestCaseError::fail(format!(
            "the rollback left {state:#?}\ninstead of {saved:#?}"
        )))
    }
}

fn effect(draft: &Draft, step: &Step, index: usize) -> Effect {
    let kind = match step {
        Step::Item(holder, id, delta) => {
            let (actor_id, place) = match holder {
                Holder::Actor(id) => (Some(id.clone()), None),
                _ => (None, Some("docks_shop".to_string())),
            };
            EffectKind::Item {
                actor_id,
                place,
                id: id.to_string(),
                delta: *delta,
                max: None,
            }
        }
        Step::Meter(actor, delta) => EffectKind::Meter {
            holder: Holder::Actor(actor.to_string()),
            key: "grit".to_string(),
            write: MeterWrite::Offset(*delta),
        },
        Step::Door(open) => EffectKind::Flag {
            holder: Holder::Exit("docks_shop".to_string(), "stairs".to_string()),
            key: "open".to_string(),
            value: *open,
        },
        Step::WorldFlag(value) => EffectKind::Flag {
            holder: Holder::World,
            key: "rung".to_string(),
            value: *value,
        },
        Step::Walk(to) => EffectKind::Location {
            actor_id: "mira".to_string(),
            from: draft.state().actors["mira"].place.clone(),
            to: to.to_string(),
            arrived_at: None,
        },
        Step::Note => EffectKind::Note {
            note_id: format!("note_rollback_{index}"),
            to: vec!["pc_rowan".to_string()],
            place: Some("docks_shop".to_string()),
            fact_ids: Vec::new(),
            text: "A test note.".to_string(),
        },
        Step::Reveal(fact) => EffectKind::Reveal {
            actor_id: "mira".to_string(),
            fact: fact.to_string(),
        },
        Step::Savepoint | Step::Rollback => unreachable!("not an effect"),
    };
    Effect::new(Some(&Viewer::Npc("mira".to_string())), "rollback", kind)
}

fn mira_intent() -> Intent {
    Intent {
        id: "rollback".to_string(),
        round_index: 0,
        actor: Some(Viewer::Npc("mira".to_string())),
        source: Source::Engine,
        move_name: "rollback".to_string(),
        target: Target::Empty,
        args: IntentArgs::default(),
        rationale_code: String::new(),
    }
}

fn pick_step() -> impl Strategy<Value = Step> {
    let holder = select(vec![
        Holder::Actor("mira".to_string()),
        Holder::Actor("bram".to_string()),
        Holder::Place("docks_shop".to_string()),
    ]);
    prop_oneof![
        3 => Just(Step::Savepoint),
        3 => Just(Step::Rollback),
        1 => (holder, select(vec!["lamp_oil", "brass_key"]), -3i64..=3)
            .prop_map(|(holder, id, delta)| Step::Item(holder, id, delta)),
        1 => (select(vec!["mira", "bram"]), -2i64..=2).prop_map(|(who, delta)| Step::Meter(who, delta)),
        1 => any::<bool>().prop_map(Step::Door),
        1 => any::<bool>().prop_map(Step::WorldFlag),
        1 => select(vec!["docks_shop", "docks_street", "mira_rooms"]).prop_map(Step::Walk),
        1 => Just(Step::Note),
        1 => select(vec!["fact_oil_on_shelf", "fact_room_lamp"]).prop_map(Step::Reveal),
    ]
}
