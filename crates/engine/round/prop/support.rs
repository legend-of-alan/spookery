//! Random legal moves, played against a temporary pack.

use std::fs;
use std::path::{Path, PathBuf};

use crate::canon::pack_rev;
use crate::ecs::component;
use crate::ecs::id::{ActorId, PlaceId};
use crate::ecs::Presence;
use crate::intent::Target;
use crate::rules::travel::visible_exits;
use crate::session;
use crate::store::journal;
use crate::store::save;
use proptest::prelude::*;
use proptest::test_runner::{Config, RngSeed, TestCaseError};

use crate::round::play;
use crate::round::replay;

pub(crate) const CASES: u32 = 64;
pub(crate) const SEED: u64 = 0x0018_0018;

const VERBS: [&str; 8] = [
    "wait", "exert", "unwield", "travel", "take", "drop", "use", "wield",
];

#[derive(Clone, Debug)]
pub(crate) struct MovePlan {
    pub pc: usize,
    pub verb: usize,
    pub token: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct Plan {
    pub moves: Vec<MovePlan>,
}

pub(crate) struct TempPack(pub PathBuf);

impl Drop for TempPack {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(crate) fn runner_config() -> Config {
    Config {
        cases: CASES,
        failure_persistence: None,
        rng_seed: RngSeed::Fixed(SEED),
        ..Config::default()
    }
}

pub(crate) fn arb_rounds() -> impl Strategy<Value = Vec<Plan>> {
    prop::collection::vec(
        prop::collection::vec(pick_move(), 0..=3).prop_map(|moves| Plan { moves }),
        1..=5,
    )
}

fn pick_move() -> impl Strategy<Value = MovePlan> {
    (0..3usize, 0..VERBS.len(), 0..8usize).prop_map(|(pc, verb, token)| MovePlan {
        pc,
        verb,
        token,
    })
}

/// Queue the moves the gate accepts, then play the round. Habits are gathered with them.
pub(crate) fn play_rounds(pack: &Path, rounds: &[Plan]) -> Result<(), String> {
    add_exert_success(pack)?;
    let lock = crate::lock_pack(pack).map_err(|err| err.to_string())?;
    let mut session = session::Session::resume(pack, lock).map_err(|err| err.to_string())?;
    let exert = VERBS
        .iter()
        .position(|verb| *verb == "exert")
        .expect("exert");
    for (index, round) in rounds.iter().enumerate() {
        for plan in &round.moves {
            queue_move(&mut session, plan);
        }
        // Every round rolls when the PC can, by `exert`.
        let pcs = pc_count(&session);
        for pc in 0..pcs {
            queue_move(
                &mut session,
                &MovePlan {
                    pc,
                    verb: exert,
                    token: 0,
                },
            );
        }
        let wall = session.world().last_round_at.clone();
        play::run_round(&mut session, &Presence::default(), &wall, false)
            .map_err(|err| format!("round {} refused to play: {err}", index + 1))?;
    }
    Ok(())
}

fn pc_count(session: &session::Session) -> usize {
    session
        .world()
        .actors()
        .query::<&component::ControlType>()
        .iter()
        .filter(|control_type| matches!(control_type, component::ControlType::Pc))
        .count()
}

/// A successful `exert` check adds one lamp oil. The docks list is empty, so the
/// roll would not otherwise change the save.
fn add_exert_success(pack: &Path) -> Result<(), String> {
    let path = pack.join("rules/verbs.yaml");
    let text = fs::read_to_string(&path).map_err(|err| err.to_string())?;
    let needle = "      threshold: 0\n    do: []\n";
    let replacement = "      threshold: 0\n    do: []\n    success:\n      - item:\n          of: self\n          id: lamp_oil\n          add: 1\n";
    if !text.contains(needle) {
        return Err("the exert check was not where the property expected".to_string());
    }
    fs::write(&path, text.replacen(needle, replacement, 1)).map_err(|err| err.to_string())?;
    let rev = pack_rev(pack).map_err(|err| err.to_string())?;
    let save_path = journal::save_path(pack);
    let save_text = fs::read_to_string(&save_path).map_err(|err| err.to_string())?;
    let mut save = save::parse(&save_text).map_err(|err| format!("{err:?}"))?;
    save.pack_rev = rev;
    fs::write(
        &save_path,
        save::emit(&save).map_err(|err| err.to_string())?,
    )
    .map_err(|err| err.to_string())
}

fn queue_move(session: &mut session::Session, plan: &MovePlan) {
    let pcs: Vec<ActorId> = session
        .world()
        .actors()
        .query::<(ActorId, &component::ControlType)>()
        .iter()
        .filter(|(_, control_type)| matches!(control_type, component::ControlType::Pc))
        .map(|(id, _)| id.clone())
        .collect();
    if pcs.is_empty() {
        return;
    }
    let actor = &pcs[plan.pc % pcs.len()];
    let verb = VERBS[plan.verb % VERBS.len()];
    let Some(target) = target_for(session, actor, verb, plan.token) else {
        return;
    };
    // A line the gate refuses, or one already queued, is left out.
    let _ = session.queue_player_move(&Presence::default(), actor, verb, target.token());
}

fn target_for(
    session: &session::Session,
    actor: &ActorId,
    verb: &str,
    token: usize,
) -> Option<Target> {
    match verb {
        "wait" | "exert" | "unwield" => Some(Target::Empty),
        "travel" => {
            let exits = visible_exits(session.canon(), session.world(), actor);
            if exits.is_empty() {
                return None;
            }
            Some(Target::Exit(exits[token % exits.len()].exit_id.clone()))
        }
        "take" | "drop" | "use" | "wield" => {
            let items = item_tokens(session, actor);
            if items.is_empty() {
                return None;
            }
            Some(Target::Item(items[token % items.len()].clone()))
        }
        _ => None,
    }
}

fn item_tokens(session: &session::Session, actor: &ActorId) -> Vec<String> {
    let mut ids: Vec<String> = session.canon().items.keys().cloned().collect();
    let world = session.world();
    if let Some(place) = world.actors().get::<component::Place>(actor).get() {
        let held = world.actors().get::<component::Item>(actor);
        ids.extend(held.iter().map(|(id, _)| id.to_string()));
        let place = PlaceId::from(place);
        let lying = world.places().get::<component::Item>(&place);
        ids.extend(lying.iter().map(|(id, _)| id.to_string()));
    }
    ids.sort();
    ids.dedup();
    ids
}

/// Replay the journal and compare it with the state hash on the last record.
pub(crate) fn assert_replay_matches(pack: &Path) -> Result<(), TestCaseError> {
    let entries = journal::read_records(pack)
        .map_err(|fault| TestCaseError::fail(format!("{}: {}", fault.file, fault.detail)))?;
    let report = replay::apply_journal(pack, &entries, None, None)
        .map_err(|err| TestCaseError::fail(err.to_string()))?;
    let Some(last) = entries.iter().rev().find(|entry| entry.changes_state()) else {
        return Err(TestCaseError::fail(
            "the rounds wrote no record".to_string(),
        ));
    };
    let replayed = crate::hash::blake2s_tagged(&report.now_yaml);
    let recorded = crate::testing::journal::state_hash(last);
    if replayed == recorded {
        return Ok(());
    }
    Err(TestCaseError::fail(format!(
        "state hash differed: record {recorded} replayed {replayed}"
    )))
}
