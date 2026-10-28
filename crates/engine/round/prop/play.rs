//! Random legal moves, played against a temporary pack.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::canon::pack_rev;
use crate::intent::{player_intent_id, ActorKind, Intent, IntentArgs, Source, Target};
use crate::journal;
use crate::map::visible;
use crate::rules::gate::gate;
use crate::store::codec::{emit_save, load_save};
use crate::world::World;
use proptest::prelude::*;
use proptest::test_runner::{Config, RngSeed, TestCaseError};

use crate::round::play::{play_round, RoundKind};
use crate::round::replay::from_journal;

pub(crate) const CASES: u32 = 64;
pub(crate) const SEED: u64 = 0x0018_0018;

const VERBS: [&str; 8] = [
    "wait", "grit", "unwield", "travel", "take", "drop", "use", "wield",
];

#[derive(Clone, Debug)]
pub(crate) struct MovePlan {
    pub pc: usize,
    pub verb: usize,
    pub token: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct RoundPlan {
    pub moves: Vec<MovePlan>,
}

pub(crate) struct TempPack(pub PathBuf);

impl Drop for TempPack {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(crate) fn configure_runner() -> Config {
    Config {
        cases: CASES,
        failure_persistence: None,
        rng_seed: RngSeed::Fixed(SEED),
        ..Config::default()
    }
}

pub(crate) fn pick_rounds() -> impl Strategy<Value = Vec<RoundPlan>> {
    prop::collection::vec(
        prop::collection::vec(pick_move(), 0..=3).prop_map(|moves| RoundPlan { moves }),
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

/// Queue the moves `gate` accepts, then play the round. Habits are gathered with them.
pub(crate) fn play_rounds(pack: &Path, rounds: &[RoundPlan]) -> Result<(), String> {
    arm_check(pack)?;
    let mut world = World::open(pack).map_err(|err| err.to_string())?;
    let grit = VERBS.iter().position(|verb| *verb == "grit").expect("grit");
    for (index, round) in rounds.iter().enumerate() {
        for plan in &round.moves {
            queue_move(&mut world, plan);
        }
        // Every round rolls when the PC can. `grit` is the pack's only check.
        let pcs = pc_count(&world);
        for pc in 0..pcs {
            queue_move(
                &mut world,
                &MovePlan {
                    pc,
                    verb: grit,
                    token: 0,
                },
            );
        }
        let wall = world.now().last_tick_at.clone();
        play_round(
            &mut world,
            &wall,
            RoundKind::Live {
                fast_forward: false,
            },
        )
        .map_err(|err| format!("round {} refused to play: {err}", index + 1))?;
    }
    Ok(())
}

pub(crate) fn copy_docks(label: &str) -> PathBuf {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packs/oneshot-docks");
    copy_pack(&source, label)
}

pub(crate) fn copy_pack(from: &Path, label: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let root =
        std::env::temp_dir().join(format!("spook-{label}-{}-{n}-{nanos}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    copy_dir(from, &root);
    root
}

fn pc_count(world: &World) -> usize {
    world
        .now()
        .actors
        .values()
        .filter(|actor| actor.is_pc())
        .count()
}

/// A successful `grit` check adds one lamp oil. The docks list is empty, so the
/// roll would not otherwise change the save.
fn arm_check(pack: &Path) -> Result<(), String> {
    let path = pack.join("rules/verbs.yaml");
    let text = fs::read_to_string(&path).map_err(|err| err.to_string())?;
    let needle = "      target: 0\n    do: []\n";
    let replacement = "      target: 0\n    do: []\n    success:\n      - item:\n          who: self\n          id: lamp_oil\n          add: 1\n";
    if !text.contains(needle) {
        return Err("the grit check was not where the property expected".to_string());
    }
    fs::write(&path, text.replacen(needle, replacement, 1)).map_err(|err| err.to_string())?;
    let rev = pack_rev(pack).map_err(|err| err.to_string())?;
    let save_path = pack.join("state/save.yaml");
    let save_text = fs::read_to_string(&save_path).map_err(|err| err.to_string())?;
    let mut now = load_save(&save_text).map_err(|err| format!("{err:?}"))?;
    now.pack_rev = rev;
    fs::write(&save_path, emit_save(&now).map_err(|err| err.to_string())?)
        .map_err(|err| err.to_string())
}

fn queue_move(world: &mut World, plan: &MovePlan) {
    let pcs: Vec<String> = world
        .now()
        .actors
        .iter()
        .filter(|(_, actor)| actor.is_pc())
        .map(|(id, _)| id.clone())
        .collect();
    if pcs.is_empty() {
        return;
    }
    let actor = &pcs[plan.pc % pcs.len()];
    let verb = VERBS[plan.verb % VERBS.len()];
    let Some(target) = target_for(world, actor, verb, plan.token) else {
        return;
    };
    let intent = player_intent(world.now(), actor, verb, target);
    if world
        .queued_intents()
        .iter()
        .any(|queued| queued.id == intent.id)
    {
        return;
    }
    if gate(world.canon(), world.now(), &intent).is_err() {
        return;
    }
    world.queue(intent);
}

fn target_for(world: &World, actor: &str, verb: &str, token: usize) -> Option<Target> {
    match verb {
        "wait" | "grit" | "unwield" => Some(Target::Empty),
        "travel" => {
            let exits = visible(world.canon(), world.now(), actor);
            if exits.is_empty() {
                return None;
            }
            Some(Target::Place(exits[token % exits.len()].name.clone()))
        }
        "take" | "drop" | "use" | "wield" => {
            let items = item_tokens(world, actor);
            if items.is_empty() {
                return None;
            }
            Some(Target::Actor(items[token % items.len()].clone()))
        }
        _ => None,
    }
}

fn item_tokens(world: &World, actor: &str) -> Vec<String> {
    let mut ids: Vec<String> = world.canon().items.keys().cloned().collect();
    if let Some(actor) = world.now().actors.get(actor) {
        ids.extend(actor.inventory().keys().cloned());
        if let Some(place) = world.now().places.get(actor.place()) {
            ids.extend(place.items.keys().cloned());
        }
    }
    ids.sort();
    ids.dedup();
    ids
}

fn player_intent(now: &crate::model::Now, actor: &str, verb: &str, target: Target) -> Intent {
    let token = match &target {
        Target::Empty => "",
        Target::Actor(id) | Target::Place(id) => id.as_str(),
        Target::Clock(_) => "",
    };
    let line = if token.is_empty() {
        verb.to_string()
    } else {
        format!("{verb} {token}")
    };
    let tick = now.round_index.saturating_add(1);
    Intent {
        id: player_intent_id(&now.shard_id, tick, actor, &line),
        round_index: now.round_index,
        actor_id: actor.to_string(),
        actor_kind: ActorKind::Player,
        source: Source::Player,
        move_name: verb.to_string(),
        target,
        args: IntentArgs::default(),
        rationale_code: String::new(),
    }
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

/// Replay the journal and compare it with the state hash on the last record.
pub(crate) fn same_save(pack: &Path) -> Result<(), TestCaseError> {
    let report = from_journal(pack, None, None).map_err(|err| fail_msg(err.to_string()))?;
    let entries = journal::read_entries(pack)
        .map_err(|fault| fail_msg(format!("{}: {}", fault.file, fault.detail)))?;
    let Some(last) = entries.iter().rev().find(|entry| entry.changes_state()) else {
        return Err(fail_msg("the rounds wrote no record".to_string()));
    };
    let replayed = journal::hash_now(&report.now_yaml);
    if replayed == last.now_hash() {
        return Ok(());
    }
    Err(fail_msg(format!(
        "state hash differed: record {} replayed {replayed}",
        last.now_hash()
    )))
}

pub(crate) fn fail_msg(message: String) -> TestCaseError {
    TestCaseError::fail(message)
}
