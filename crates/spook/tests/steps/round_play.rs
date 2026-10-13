use std::fs;
use std::path::{Path, PathBuf};

use cucumber::{given, then, when};
use serde_json::Value;
use spookery_kernel::intent::{ActorKind, Intent, IntentArgs, Source, Target};
use spookery_kernel::rules::RollReport;
use spookery_round::TickCause;

use crate::support::{self, copy_dir};
use crate::World;

#[given(expr = "every existing cucumber feature")]
fn every_existing_cucumber_feature(world: &mut World) {
    let features = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/features");
    let mut count = 0usize;
    walk_features(&features, &mut count);
    assert!(count > 1, "the suite has one feature file");
    world.scratch = count.to_string();
}

#[then(expr = "all pass and spook verify passes on both sample packs")]
fn all_pass_and_spook_verify_passes_on_both_sample_packs(_world: &mut World) {
    for name in ["oneshot-docks", "sample-d20"] {
        let pack = copy_named(name, "verify-clean");
        let output = support::spook(&["verify", &pack.to_string_lossy()]);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "{name} verify failed\n{stdout}{stderr}"
        );
        let _ = fs::remove_dir_all(pack);
    }
}

#[then(
    expr = "crates\\/round has no call to resolve_check and kernel rules::roll is the only roll function"
)]
fn crates_round_has_no_call_to_resolve_check(world: &mut World) {
    let root = workspace_root();
    let round = root.join("crates/round");
    let mut files = Vec::new();
    files_under(&round, &mut files);
    for path in &files {
        let text = fs::read_to_string(path).unwrap();
        assert!(
            !text.contains("resolve_check("),
            "{} calls resolve_check",
            path.display()
        );
    }
    let kernel = root.join("crates/kernel");
    let mut kernel_files = Vec::new();
    files_under(&kernel, &mut kernel_files);
    let mut rolls = Vec::new();
    for path in &kernel_files {
        let text = fs::read_to_string(path).unwrap();
        assert!(!text.contains("fn resolve_check"), "{}", path.display());
        assert!(!text.contains("fn die_face"), "{}", path.display());
        if text.contains("pub fn roll(") {
            rolls.push(path.clone());
        }
    }
    assert_eq!(rolls.len(), 1, "{rolls:?}");
    assert!(rolls[0].ends_with("rules/roll.rs") || rolls[0].ends_with("rules\\roll.rs"));
    world.scratch = "one-roll".to_string();
    assert_round_names(&root);
}

#[when(expr = "a docks tick is played")]
fn a_docks_tick_is_played(world: &mut World) {
    let pack = copy_named("oneshot-docks", "stage-order");
    spookery_round::play_pack(&pack).unwrap_or_else(|err| panic!("play: {err}"));
    world.pack = Some(pack);
}

#[then(
    expr = "the tick trace lists advance, complete, gather, resolve, apply, run_triggers, observe, record in that order"
)]
fn the_tick_trace_lists_the_stages(world: &mut World) {
    let tick = tick_value(world.pack.as_ref().unwrap(), "tick-0001.jsonl");
    let trace: Vec<&str> = tick["tool_trace"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let stages = [
        "advance",
        "complete",
        "gather",
        "resolve",
        "apply",
        "run_triggers",
        "observe",
        "record",
    ];
    let mut at = 0usize;
    for name in &trace {
        if at < stages.len() && *name == stages[at] {
            at += 1;
        }
    }
    assert_eq!(at, stages.len(), "{trace:?}");
}

#[given(expr = "a tick where Lark rolls grit")]
fn a_tick_where_lark_rolls_grit(world: &mut World) {
    let clean = lark_pack();
    let played = copy_tree(&clean, "lark-alone");
    let (faces, total) = lark_roll(&played, false);
    let _ = fs::remove_dir_all(played);
    world.scratch = format!("{faces}|{total}");
    world.pack = Some(clean);
}

#[when(expr = "the same tick is replayed with an extra unrelated NPC roll queued first")]
fn the_same_tick_is_replayed_with_an_extra_roll(world: &mut World) {
    let pack = world.pack.as_ref().unwrap().clone();
    let (faces, total) = lark_roll(&pack, true);
    world.player_added = format!("{faces}|{total}");
}

#[then(expr = "Lark's faces and total are identical")]
fn larks_faces_and_total_are_identical(world: &mut World) {
    assert_eq!(world.player_added, world.scratch);
}

#[given(expr = "ten rolling intents in one tick")]
fn ten_rolling_intents_in_one_tick(world: &mut World) {
    let pack = copy_named("oneshot-docks", "ten-dice");
    let intents = (0..10)
        .map(|index| {
            queued(
                &format!("grit-{index}"),
                "pc_rowan",
                ActorKind::Player,
                "grit",
                Target::Empty,
            )
        })
        .collect();
    let (_now, rolls, tick) = play_queued(&pack, intents);
    world.scratch = tick.to_string();
    world.player_added = rolls.len().to_string();
    world.pack = Some(pack);
}

#[then(expr = "all ten resolve and none is refused NotInSpec")]
fn all_ten_resolve_and_none_is_refused_not_in_spec(world: &mut World) {
    assert_eq!(world.player_added, "10");
    let tick: Value = serde_json::from_str(&world.scratch).unwrap();
    let rejected = tick["rejected"].as_array().unwrap();
    assert!(
        rejected.iter().all(|item| item["reason"] != "NOT_IN_SPEC"),
        "{rejected:?}"
    );
}

#[given(expr = "a played d20 fight")]
fn a_played_d20_fight(world: &mut World) {
    let pack = copy_named("sample-d20", "d20-fight");
    let intents = vec![queued(
        "strike-lark",
        "pc_lark",
        ActorKind::Player,
        "strike",
        Target::Actor("npc_briar".to_string()),
    )];
    let (now, _rolls, _tick) = play_queued(&pack, intents);
    world.now_bytes = spookery_kernel::codec::emit_now(&now).unwrap().into_bytes();
    world.pack = Some(pack);
}

#[then(expr = "spook verify passes and re-derives every roll")]
fn spook_verify_passes_and_re_derives_every_roll(world: &mut World) {
    let pack = world.pack.as_ref().unwrap();
    let output = support::spook(&["verify", &pack.to_string_lossy()]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}{stderr}");
    let report =
        spookery_round::replay_journal(pack, None, None).unwrap_or_else(|err| panic!("{err:?}"));
    assert_eq!(
        report.now_yaml,
        fs::read(pack.join("state/now.yaml")).unwrap()
    );
}

#[then(expr = "the first effect is absent from the save and the tick log names the refusal")]
fn the_first_effect_is_absent_from_the_save(world: &mut World) {
    let pack = copy_named("oneshot-docks", "rollback");
    append(
        &pack.join("rules/moves.yaml"),
        "
  - name: overreach
    audience: player
    token: none
    on: speaker
    effects:
      - class: stock
        place: docks_shop
        sku: lamp_oil
        delta: 1
      - class: stock
        place: docks_shop
        sku: lamp_oil
        delta: -8
",
    );
    stamp(&pack);
    set_mira_busy(&pack);
    let (now, _rolls, tick) = play_queued(
        &pack,
        vec![queued(
            "overreach-1",
            "mira",
            ActorKind::Npc,
            "overreach",
            Target::Empty,
        )],
    );
    let oil = now
        .places
        .get("docks_shop")
        .and_then(|place| place.stock.get("lamp_oil"))
        .copied();
    assert_eq!(oil, Some(4), "{now:?}");
    let rejected = tick["rejected"].as_array().unwrap();
    assert!(
        rejected
            .iter()
            .any(|item| item["id"] == "overreach-1" && item["reason"].as_str().is_some()),
        "{tick}"
    );
    assert!(tick["effects"].as_array().unwrap().is_empty(), "{tick}");
    world.pack = Some(pack);
}

#[given(expr = "max_effects_per_intent is 1 and a row with 2 effects")]
fn max_effects_per_intent_is_1_and_a_row_with_2_effects(world: &mut World) {
    let pack = copy_named("oneshot-docks", "effect-cap");
    let tick_path = pack.join("rules/tick.yaml");
    let text = fs::read_to_string(&tick_path).unwrap();
    let next = text.replacen("max_effects_per_intent: 4", "max_effects_per_intent: 1", 1);
    assert_ne!(next, text, "cap was not lowered");
    fs::write(&tick_path, next).unwrap();
    append(
        &pack.join("rules/moves.yaml"),
        "
  - name: two_stock
    audience: player
    token: none
    on: speaker
    effects:
      - class: stock
        place: docks_shop
        sku: lamp_oil
        delta: 1
      - class: stock
        place: docks_shop
        sku: lamp_oil
        delta: 1
",
    );
    stamp(&pack);
    set_mira_busy(&pack);
    let (now, _rolls, tick) = play_queued(
        &pack,
        vec![queued(
            "two-1",
            "mira",
            ActorKind::Npc,
            "two_stock",
            Target::Empty,
        )],
    );
    world.now_bytes = format!(
        "{}",
        now.places
            .get("docks_shop")
            .and_then(|place| place.stock.get("lamp_oil"))
            .copied()
            .unwrap_or(-1)
    )
    .into_bytes();
    world.scratch = tick.to_string();
    world.pack = Some(pack);
}

#[then(expr = "no effect from that row applies and the tick log names the refusal")]
fn no_effect_from_that_row_applies(world: &mut World) {
    assert_eq!(String::from_utf8_lossy(&world.now_bytes), "4");
    let tick: Value = serde_json::from_str(&world.scratch).unwrap();
    assert!(tick["effects"].as_array().unwrap().is_empty(), "{tick}");
    let rejected = tick["rejected"].as_array().unwrap();
    assert!(
        rejected
            .iter()
            .any(|item| item["id"] == "two-1" && item["reason"] == "budget"),
        "{tick}"
    );
}

fn assert_round_names(root: &Path) {
    assert!(!root.join("crates/tick").exists());
    let mut banned = Vec::new();
    for dir in ["crates", "docs"] {
        let mut files = Vec::new();
        files_under(&root.join(dir), &mut files);
        for path in files {
            if path.extension().and_then(|ext| ext.to_str()) == Some("md")
                || path.extension().and_then(|ext| ext.to_str()) == Some("rs")
            {
                let text = fs::read_to_string(&path).unwrap_or_default();
                for needle in [
                    concat!("die_", "cursor"),
                    concat!("stage_", "tick"),
                    concat!("spookery_", "tick"),
                ] {
                    if text.contains(needle) {
                        banned.push(format!("{} contains {needle}", path.display()));
                    }
                }
            }
        }
    }
    assert!(banned.is_empty(), "{}", banned.join("\n"));
    let world = fs::read_to_string(root.join("crates/kernel/world.rs")).unwrap();
    for name in ["set_pending", "set_rolls", "queue_intent", "clear_queue"] {
        assert!(
            !world.lines().any(|line| {
                let trimmed = line.trim();
                trimmed.starts_with(&format!("pub fn {name}("))
            }),
            "World publishes {name}"
        );
    }
    let mut round_files = Vec::new();
    files_under(&root.join("crates/round"), &mut round_files);
    for path in round_files {
        let text = fs::read_to_string(&path).unwrap();
        for line in text.lines() {
            let trimmed = line.trim();
            let Some(rest) = trimmed.strip_prefix("pub fn ") else {
                continue;
            };
            let name: String = rest
                .chars()
                .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
                .collect();
            assert!(!name.contains("turn"), "{} pub fn {name}", path.display());
        }
    }
}

fn lark_pack() -> PathBuf {
    let pack = copy_named("sample-d20", "lark-grit");
    append(
        &pack.join("rules/meters.yaml"),
        "
  - id: grit
    min: 0
    base_cap: 100
    max: 110
    start: 1
",
    );
    append(
        &pack.join("rules/checks.yaml"),
        "
  - id: grit
    dice:
      count: 2
      sides: 6
    stat: grit
",
    );
    append(
        &pack.join("rules/moves.yaml"),
        "
  - name: grit
    audience: player
    token: none
    on: speaker
    check: grit
    difficulty: 0
    effects: []
",
    );
    stamp(&pack);
    pack
}

fn lark_roll(pack: &Path, extra_first: bool) -> (String, i64) {
    let mut intents = Vec::new();
    if extra_first {
        intents.push(queued(
            "briar-strike",
            "npc_briar",
            ActorKind::Npc,
            "strike",
            Target::Actor("npc_oden".to_string()),
        ));
    }
    intents.push(queued(
        "lark-grit",
        "pc_lark",
        ActorKind::Player,
        "grit",
        Target::Empty,
    ));
    let (_now, rolls, _tick) = play_queued(pack, intents);
    let rolled = rolls.into_iter().find_map(|notice| match notice {
        RollReport::Rolled { actor_id, out } if actor_id == "pc_lark" => Some((
            out.faces
                .iter()
                .map(|face| face.to_string())
                .collect::<Vec<_>>()
                .join(","),
            out.total,
        )),
        _ => None,
    });
    rolled.unwrap_or_else(|| panic!("Lark did not roll"))
}

fn play_queued(
    pack: &Path,
    intents: Vec<Intent>,
) -> (spookery_kernel::model::Now, Vec<RollReport>, Value) {
    let mut world = spookery_kernel::World::open(pack).unwrap_or_else(|err| panic!("open: {err}"));
    let wall = world.now().last_tick_at.clone();
    for intent in intents {
        world.queue(intent);
    }
    spookery_round::play(&mut world, TickCause::Manual, &wall)
        .unwrap_or_else(|err| panic!("play: {err}"));
    let tick = tick_value(pack, "tick-0001.jsonl");
    let now = world.now().clone();
    let rolls = world.take_rolls();
    (now, rolls, tick)
}

fn queued(id: &str, actor: &str, kind: ActorKind, move_name: &str, target: Target) -> Intent {
    Intent {
        id: id.to_string(),
        tick_index: 1,
        actor_id: actor.to_string(),
        actor_kind: kind,
        source: Source::Player,
        move_name: move_name.to_string(),
        target,
        args: IntentArgs::default(),
        rationale_code: "test".to_string(),
    }
}

fn set_mira_busy(pack: &Path) {
    let path = pack.join("state/now.yaml");
    let mut now = spookery_kernel::codec::load_now(&fs::read_to_string(&path).unwrap()).unwrap();
    *now.actors.get_mut("mira").unwrap().busy_mut() = true;
    fs::write(&path, spookery_kernel::codec::emit_now(&now).unwrap()).unwrap();
}

fn stamp(pack: &Path) {
    let now_path = pack.join("state/now.yaml");
    let mut now =
        spookery_kernel::codec::load_now(&fs::read_to_string(&now_path).unwrap()).unwrap();
    now.pack_rev = spookery_kernel::canon::pack_rev(pack).unwrap();
    fs::write(&now_path, spookery_kernel::codec::emit_now(&now).unwrap()).unwrap();
}

fn append(path: &Path, extra: &str) {
    let mut text = fs::read_to_string(path).unwrap();
    text.push_str(extra);
    fs::write(path, text).unwrap();
}

fn tick_value(pack: &Path, name: &str) -> Value {
    let text =
        fs::read_to_string(pack.join("state/log").join(name)).unwrap_or_else(|err| panic!("{err}"));
    serde_json::from_str(text.trim_end()).unwrap_or_else(|err| panic!("{err}: {text}"))
}

fn copy_named(name: &str, label: &str) -> PathBuf {
    let source = workspace_root().join("packs").join(name);
    let pack = std::env::temp_dir().join(format!(
        "spook-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&pack);
    copy_dir(&source, &pack);
    pack
}

fn copy_tree(from: &Path, label: &str) -> PathBuf {
    let pack = std::env::temp_dir().join(format!(
        "spook-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&pack);
    copy_dir(from, &pack);
    pack
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn walk_features(dir: &Path, count: &mut usize) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            walk_features(&path, count);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("feature") {
            *count += 1;
        }
    }
}

fn files_under(dir: &Path, out: &mut Vec<PathBuf>) {
    if !dir.exists() {
        return;
    }
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files_under(&path, out);
        } else {
            out.push(path);
        }
    }
}
