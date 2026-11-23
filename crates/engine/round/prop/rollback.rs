//! The scenario this property checks:
//!
//! ```gherkin
//! Feature: A rollback puts the save back exactly
//!
//!   Scenario: Random writes to every store under nested savepoints roll back to each savepoint's save
//!     Given the docks pack, with Mira holding one brass key, a flag Mira can carry, two flags and a meter the street can carry, a story with a meter and a flag, a world meter, and a fatigue on vigor
//!     When random effects on every store, the actor's place, wielded item, visited places and action state included, land under savepoints opened and rolled back at random, Mira's memory of the shop's exits written over among them
//!     Then each rollback leaves the save and the counted keys as they were when its savepoint was taken
//!     And each take reports every meter whose value moved since the last take
//!     And rolling back to the first savepoint leaves no change to report
//! ```
//!
//! Deterministic in CI: 64 cases, seed `0x0018_0018`, no failure file.

use std::sync::Arc;

use crate::canon::StoryDef;
use crate::ecs::change::{self, Change, ChangeValue};
use crate::ecs::component::meter::{self, Meter};
use crate::ecs::component::modifier;
use crate::ecs::component::{self, Component};
use crate::ecs::id::{ActorId, ExitId};
use crate::ecs::{self, Presence, Save, WindowCount};
use crate::entity;
use crate::intent::{Intent, IntentArgs, Source, Target};
use crate::rules::effect::{Effect, EffectKind};
use crate::session;
use crate::testing::copy_pack;

use super::support::{runner_config, TempPack};
use proptest::prelude::*;
use proptest::sample::select;
use proptest::test_runner::{TestCaseError, TestRunner};

#[derive(Clone, Debug)]
enum Step {
    Savepoint,
    Rollback,
    Take,
    Item(entity::Id, &'static str, i64),
    Meter(entity::Id, &'static str, i64),
    Fatigue(i64),
    Expire,
    ExitOpen(bool),
    Flag(entity::Id, &'static str, bool),
    Walk(&'static str),
    Wield(&'static str),
    Unwield,
    Note,
    Act(u32),
    ResetIdleWindowCount,
    Busy(bool),
    NoteExits,
    SeeExits(bool),
}

#[test]
fn nested_rollbacks_put_the_save_back_exactly() {
    let pack = TempPack(copy_pack("docks", "rollback"));
    let path = pack.0.join("state/save.yaml");
    let text = std::fs::read_to_string(&path).expect("the docks save");
    assert!(
        !text.contains("\n    docks_street:"),
        "the street holds something"
    );
    let held = "\n  items:\n    harbormouse:\n      tonic: 1\n";
    let actor_items = &format!("{held}    mira:\n      brass_key: 1\n");
    let edited = text.replacen(held, actor_items, 1);
    assert!(edited.contains(actor_items), "Mira holds no key");
    let edited = crate::store::save::parse(&edited).expect("the edited save reads");
    let edited = crate::store::save::emit(&edited).expect("the edited save writes");
    std::fs::write(&path, edited).expect("write the save");
    let opened = session::Session::open(&pack.0).expect("docks pack opens");
    let mut canon = opened.canon().clone();
    canon.budget.max_state_keys_mutated = 1000;
    let key = canon.items.get_mut("brass_key").expect("the brass key");
    key.tags.push("weapon".to_string());
    let tale = StoryDef {
        name: "tale".to_string(),
    };
    canon.stories.insert("tale".into(), tale);
    let fatigue = modifier::Descriptor {
        meter: "vigor".to_string(),
        max_stacks: 2,
        hidden: false,
    };
    canon.modifiers.insert("fatigue".to_string(), fatigue);
    let carried = [
        (entity::Id::Place("docks_street".into()), "glow"),
        (entity::Id::Story("tale".into()), "tension"),
        (entity::Id::Global, "tide"),
    ];
    for (carrier, key) in &carried {
        let desc = meter::Descriptor {
            name: String::new(),
            min: 0,
            base_cap: 10,
            max: 10,
            base: 5,
            hidden: false,
            carrier: Some(carrier.clone()),
        };
        match carrier {
            entity::Id::Global => canon.world_meters.insert(key.to_string(), desc),
            _ => canon.meters.insert(key.to_string(), desc),
        };
    }
    let mut world = ecs::World::new(Save::clone(opened.world()), Arc::new(canon));
    for (carrier, key) in &carried {
        let desc = world.canon().meter_desc(carrier.as_ref(), key);
        let base = Meter::at_base(desc.expect("the meter's descriptor"));
        let put = (world.entities_mut().get_mut::<Meter>(carrier)).set(key, Some(base));
        put.expect("the meter goes on");
    }
    world.accept();
    let session = session::Session::from_parts(&pack.0, world);
    let mut runner = TestRunner::new(runner_config());
    runner
        .run(&prop::collection::vec(pick_step(), 1..40), |steps| {
            play_steps(&session, &steps)
        })
        .unwrap_or_else(|err| panic!("rollback property failed: {err}"));
}

/// A savepoint, with the save and the counted keys as they were when it was taken.
struct Open {
    point: crate::ecs::Savepoint,
    saved: ecs::World,
    keys: usize,
}

fn play_steps(session: &session::Session<()>, steps: &[Step]) -> Result<(), TestCaseError> {
    let nobody = Presence::default();
    let canon = session.canon();
    let mut world = session.begin_round();
    let intent = mira_intent();
    let mira = ActorId::from("mira");
    let open_at = |world: &ecs::World| Open {
        point: world.savepoint(),
        saved: world.clone(),
        keys: world.keys_counted(),
    };
    let mut open = vec![open_at(&world)];
    let mut taken = world.clone();
    for (index, step) in steps.iter().enumerate() {
        match step {
            Step::Savepoint => open.push(open_at(&world)),
            Step::Rollback if open.len() > 1 => {
                let at = open.pop().expect("an open savepoint");
                roll_back(&mut world, at)?;
            }
            Step::Rollback => {}
            Step::Take if open.len() == 1 => {
                let changes = world.take_changes();
                every_moved_meter_is_reported(&taken, &world, &changes)?;
                taken = world.clone();
            }
            Step::Take => {}
            Step::Expire => {
                let _ = world.actors_mut().expire_modifiers(u64::MAX);
            }
            Step::Act(players) => {
                let _ = world
                    .actors_mut()
                    .get_mut::<component::Act>(&mira)
                    .count(1, *players);
            }
            Step::ResetIdleWindowCount => {
                let _ = (world.actors_mut()).reset_window_count(WindowCount::Idle);
            }
            Step::Busy(true) => {
                let _ = world
                    .actors_mut()
                    .get_mut::<component::Act>(&mira)
                    .set_busy(9, None);
            }
            Step::Busy(false) => {
                let _ = world
                    .actors_mut()
                    .get_mut::<component::Act>(&mira)
                    .take_pending();
            }
            Step::NoteExits => {
                let _ = crate::rules::travel::note_exits(canon, &mut world);
            }
            Step::SeeExits(both) => {
                let exits = ["docks_street", "stairs"][..if *both { 2 } else { 1 }].iter();
                let exits = exits.map(|exit| exit.to_string()).collect();
                let mut actors = world.actors_mut();
                let _ = (actors.get_mut::<component::Visit>(&mira)).set("docks_shop", exits);
            }
            _ => {
                let effect = effect(&world, step, index);
                let _ = effect.apply(canon, &mut world, &nobody, &intent);
            }
        }
    }
    while let Some(at) = open.pop() {
        roll_back(&mut world, at)?;
    }
    if !world.take_changes().is_empty() {
        return Err(TestCaseError::fail("a rolled-back write is still reported"));
    }
    Ok(())
}

fn roll_back(world: &mut ecs::World, at: Open) -> Result<(), TestCaseError> {
    world.rollback(at.point);
    if **world != *at.saved {
        return Err(TestCaseError::fail(format!(
            "the rollback left {:#?}\ninstead of {:#?}",
            world, at.saved
        )));
    }
    if world.keys_counted() != at.keys {
        return Err(TestCaseError::fail(format!(
            "the rollback left {} keys counted instead of {}",
            world.keys_counted(),
            at.keys
        )));
    }
    Ok(())
}

/// Every actor meter whose value differs between `taken` and `now` is in `changes`, with the
/// value it had at the last take and the value it has now.
fn every_moved_meter_is_reported(
    taken: &ecs::World,
    now: &ecs::World,
    changes: &[Change],
) -> Result<(), TestCaseError> {
    let read = |world: &ecs::World, actor_id: &ActorId, key: &str| {
        let value = modifier::value(world.actors(), actor_id, key);
        value.ok().map(ChangeValue::Number)
    };
    let meters = now.actors().query::<&Meter>();
    for (actor_id, key, _) in meters.iter().flat_map(|held| held.iter_with_id()) {
        let (before, after) = (read(taken, actor_id, key), read(now, actor_id, key));
        if before == after {
            continue;
        }
        let moved = Change {
            key: change::Key::new(Meter::ID, entity::IdRef::Actor(actor_id), Some(key)),
            before,
            after,
        };
        if !changes.contains(&moved) {
            return Err(TestCaseError::fail(format!(
                "{moved:?} is missing from {changes:?}"
            )));
        }
    }
    Ok(())
}

fn effect(world: &ecs::World, step: &Step, index: usize) -> Effect {
    let mira = ActorId::from("mira");
    let kind = match step {
        Step::Item(carrier, id, delta) => {
            let (actor_id, place) = match carrier {
                entity::Id::Actor(id) => (Some(id.clone()), None),
                _ => (None, Some("docks_shop".into())),
            };
            EffectKind::Item {
                actor_id,
                place,
                id: id.to_string(),
                delta: *delta,
                max: None,
            }
        }
        Step::Meter(carrier, key, delta) => EffectKind::Meter {
            entity: carrier.clone(),
            key: key.to_string(),
            write: meter::Write::Offset(*delta),
        },
        Step::Fatigue(add) => EffectKind::Modifier {
            actor_id: mira,
            id: "fatigue".to_string(),
            add: *add,
            ticks: 3,
        },
        Step::ExitOpen(open) => EffectKind::ExitOpen {
            exit: ExitId {
                place: "docks_shop".into(),
                exit: "stairs".to_string(),
            },
            open: *open,
        },
        Step::Flag(carrier, key, value) => EffectKind::Flag {
            entity: carrier.clone(),
            key: key.to_string(),
            value: *value,
        },
        Step::Walk(to) => EffectKind::Location {
            from: (world.actors().get::<component::Place>(&mira).get())
                .map(String::from)
                .unwrap_or_default(),
            actor_id: mira,
            to: to.to_string(),
            arrived_at: None,
        },
        Step::Wield(id) => EffectKind::Wield {
            actor_id: mira,
            id: id.to_string(),
        },
        Step::Unwield => EffectKind::Unwield { actor_id: mira },
        Step::Note => EffectKind::Note {
            note_id: format!("note_rollback_{index}"),
            to_ids: vec!["rowan".into()],
            place_id: Some("docks_shop".into()),
            text: "A test note.".to_string(),
        },
        Step::Savepoint
        | Step::Rollback
        | Step::Take
        | Step::Expire
        | Step::Act(_)
        | Step::ResetIdleWindowCount
        | Step::Busy(_)
        | Step::NoteExits
        | Step::SeeExits(_) => {
            unreachable!("not an effect")
        }
    };
    Effect::new(Some(&"mira".into()), "rollback", kind)
}

fn mira_intent() -> Intent {
    Intent {
        id: "rollback".to_string(),
        round_index: 0,
        actor_id: Some("mira".into()),
        source: Source::Engine,
        move_id: "rollback".to_string(),
        target: Target::Empty,
        args: IntentArgs::default(),
        rationale_code: String::new(),
    }
}

fn pick_step() -> impl Strategy<Value = Step> {
    let carrier = select(vec![
        entity::Id::Actor("mira".into()),
        entity::Id::Actor("bram".into()),
        entity::Id::Place("docks_shop".into()),
    ]);
    let meters = vec![
        (entity::Id::Actor("mira".into()), "vigor"),
        (entity::Id::Actor("bram".into()), "vigor"),
        (entity::Id::Place("docks_street".into()), "glow"),
        (entity::Id::Story("tale".into()), "tension"),
        (entity::Id::Global, "tide"),
    ];
    let flags = vec![
        (entity::Id::Actor("mira".into()), "wary"),
        (entity::Id::Place("docks_street".into()), "lit"),
        (entity::Id::Place("docks_street".into()), "busy"),
        (entity::Id::Story("tale".into()), "told"),
        (entity::Id::Global, "rung"),
    ];
    prop_oneof![
        3 => Just(Step::Savepoint),
        3 => Just(Step::Rollback),
        2 => Just(Step::Take),
        1 => (carrier, select(vec!["lamp_oil", "brass_key"]), -3i64..=3)
            .prop_map(|(carrier, id, delta)| Step::Item(carrier, id, delta)),
        2 => (select(meters), -2i64..=2)
            .prop_map(|((carrier, key), delta)| Step::Meter(carrier, key, delta)),
        1 => (-2i64..=2).prop_map(Step::Fatigue),
        1 => Just(Step::Expire),
        1 => any::<bool>().prop_map(Step::ExitOpen),
        2 => (select(flags), any::<bool>())
            .prop_map(|((carrier, key), value)| Step::Flag(carrier, key, value)),
        1 => select(vec!["docks_shop", "docks_street", "mira_rooms"]).prop_map(Step::Walk),
        1 => select(vec!["brass_key", "lamp_oil"]).prop_map(Step::Wield),
        1 => Just(Step::Unwield),
        1 => Just(Step::Note),
        1 => (0u32..=1).prop_map(Step::Act),
        1 => Just(Step::ResetIdleWindowCount),
        1 => any::<bool>().prop_map(Step::Busy),
        1 => Just(Step::NoteExits),
        1 => any::<bool>().prop_map(Step::SeeExits),
    ]
}
