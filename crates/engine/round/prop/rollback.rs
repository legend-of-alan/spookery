//! The scenario this property checks:
//!
//! ```gherkin
//! Feature: A rollback puts the save back exactly
//!
//!   Scenario: Random writes under nested savepoints roll back to each savepoint's save
//!     Given the docks pack, with no live record yet for the street, and a burn on grit
//!     When random effects and round writes land under savepoints opened and rolled back at random
//!     Then each rollback leaves the save and the spent keys as they were when its savepoint was taken
//!     And each take reports every meter whose value moved since the last take
//!     And rolling back to the first savepoint leaves no change to report
//! ```
//!
//! Deterministic in CI: 64 cases, seed `0x0018_0018`, no failure file.

use std::sync::Arc;

use crate::draft::Draft;
use crate::intent::{Intent, IntentArgs, Source, Target, Viewer};
use crate::model::key::{Change, StateKey, StateValue};
use crate::model::meter::MeterWrite;
use crate::model::modifier::ModifierDef;
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
    Take,
    Item(Holder, &'static str, i64),
    Meter(&'static str, i64),
    Burn(i64),
    Expire,
    Door(bool),
    StreetFlag(bool),
    WorldFlag(bool),
    Walk(&'static str),
    Note,
    Reveal(&'static str),
    Act,
    Busy(bool),
}

#[test]
fn nested_rollbacks_put_the_save_back_exactly() {
    let pack = TempPack(copy_docks("rollback"));
    let opened = World::open(&pack.0).expect("docks pack opens");
    // The street has no live record yet, so walking there or lighting it gives it its first one.
    let mut state = opened.state().clone();
    state.places.remove("docks_street");
    let mut canon = opened.canon().clone();
    let street = canon.places.get_mut("docks_street").expect("the street");
    street.flags.push("lit".to_string());
    let burn = ModifierDef {
        meter: "grit".to_string(),
        max_stacks: 2,
        hidden: false,
    };
    canon.modifiers.insert("burn".to_string(), burn);
    let world = World::from_parts(&pack.0, Arc::new(canon), state);
    let mut runner = TestRunner::new(runner_config());
    runner
        .run(&prop::collection::vec(pick_step(), 1..40), |steps| {
            play_steps(&world, &steps)
        })
        .unwrap_or_else(|err| panic!("rollback property failed: {err}"));
}

/// A savepoint, with the save and the spent keys as they were when it was taken.
struct Open {
    point: crate::draft::Savepoint,
    saved: State,
    keys: usize,
}

fn play_steps(world: &World<()>, steps: &[Step]) -> Result<(), TestCaseError> {
    let nobody = crate::model::Presence::default();
    let mut draft = Draft::begin(world, &nobody);
    let intent = mira_intent();
    let open_at = |draft: &Draft| Open {
        point: draft.savepoint(),
        saved: draft.state().clone(),
        keys: draft.keys_spent(),
    };
    let mut open = vec![open_at(&draft)];
    let mut taken = draft.state().clone();
    for (index, step) in steps.iter().enumerate() {
        // A refused write is left for a savepoint to undo, as in a round.
        match step {
            Step::Savepoint => open.push(open_at(&draft)),
            Step::Rollback if open.len() > 1 => {
                let at = open.pop().expect("an open savepoint");
                roll_back(&mut draft, at)?;
            }
            Step::Rollback => {}
            // A round takes changes only between actions, outside any savepoint but the first.
            Step::Take if open.len() == 1 => {
                let changes = draft.take_changes();
                every_moved_meter_is_reported(world, &taken, draft.state(), &changes)?;
                taken = draft.state().clone();
            }
            Step::Take => {}
            Step::Expire => {
                let _ = draft.expire_modifiers(u64::MAX);
            }
            Step::Act => {
                let _ = draft.count_act("mira");
            }
            Step::Busy(true) => {
                let _ = draft.set_busy("mira", 9, None);
            }
            Step::Busy(false) => {
                let _ = draft.take_pending("mira");
            }
            _ => {
                let _ = effect(&draft, step, index).apply(&mut draft, &intent);
            }
        }
    }
    while let Some(at) = open.pop() {
        roll_back(&mut draft, at)?;
    }
    if !draft.take_changes().is_empty() {
        return Err(TestCaseError::fail("a rolled-back write is still reported"));
    }
    Ok(())
}

fn roll_back(draft: &mut Draft, at: Open) -> Result<(), TestCaseError> {
    draft.rollback(at.point);
    if draft.state() != &at.saved {
        return Err(TestCaseError::fail(format!(
            "the rollback left {:#?}\ninstead of {:#?}",
            draft.state(),
            at.saved
        )));
    }
    if draft.keys_spent() != at.keys {
        return Err(TestCaseError::fail(format!(
            "the rollback left {} keys spent instead of {}",
            draft.keys_spent(),
            at.keys
        )));
    }
    Ok(())
}

/// Every actor meter whose value differs between `taken` and `now` is in `changes`, with the
/// value it had at the last take and the value it has now.
fn every_moved_meter_is_reported(
    world: &World<()>,
    taken: &State,
    now: &State,
    changes: &[Change],
) -> Result<(), TestCaseError> {
    let read = |state: &State, holder: &Holder, key: &str| {
        state
            .meter_value(world.canon(), holder, key)
            .map_or(StateValue::Empty, StateValue::Number)
    };
    for (id, actor) in &now.actors {
        let holder = Holder::Actor(id.clone());
        for key in actor.meters.keys() {
            let (before, after) = (read(taken, &holder, key), read(now, &holder, key));
            if before == after {
                continue;
            }
            let moved = Change {
                key: StateKey::ActorMeter(id.clone(), key.clone()),
                before,
                after,
            };
            if !changes.contains(&moved) {
                return Err(TestCaseError::fail(format!(
                    "{moved:?} is missing from {changes:?}"
                )));
            }
        }
    }
    Ok(())
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
        Step::Burn(add) => EffectKind::Modifier {
            actor_id: "mira".to_string(),
            id: "burn".to_string(),
            add: *add,
            ticks: 3,
        },
        Step::Door(open) => EffectKind::Flag {
            holder: Holder::Exit("docks_shop".to_string(), "stairs".to_string()),
            key: "open".to_string(),
            value: *open,
        },
        Step::StreetFlag(value) => EffectKind::Flag {
            holder: Holder::Place("docks_street".to_string()),
            key: "lit".to_string(),
            value: *value,
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
        Step::Savepoint
        | Step::Rollback
        | Step::Take
        | Step::Expire
        | Step::Act
        | Step::Busy(_) => {
            unreachable!("not an effect")
        }
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
        2 => Just(Step::Take),
        1 => (holder, select(vec!["lamp_oil", "brass_key"]), -3i64..=3)
            .prop_map(|(holder, id, delta)| Step::Item(holder, id, delta)),
        1 => (select(vec!["mira", "bram"]), -2i64..=2).prop_map(|(who, delta)| Step::Meter(who, delta)),
        1 => (-2i64..=2).prop_map(Step::Burn),
        1 => Just(Step::Expire),
        1 => any::<bool>().prop_map(Step::Door),
        1 => any::<bool>().prop_map(Step::StreetFlag),
        1 => any::<bool>().prop_map(Step::WorldFlag),
        1 => select(vec!["docks_shop", "docks_street", "mira_rooms"]).prop_map(Step::Walk),
        1 => Just(Step::Note),
        1 => select(vec!["fact_oil_on_shelf", "fact_room_lamp"]).prop_map(Step::Reveal),
        1 => Just(Step::Act),
        1 => any::<bool>().prop_map(Step::Busy),
    ]
}
