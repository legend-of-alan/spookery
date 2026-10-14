use std::fs;
use std::path::{Path, PathBuf};

use cucumber::{given, then, when};
use serde_json::Value;
use spookery_kernel::intent::{ActorKind, Intent, IntentArgs, Source, Target, ViewerId};
use spookery_kernel::rules::{roll, RollInput, RollKey, RollReport};
use spookery_round::RoundCause;

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

#[then(expr = "crates\\/round has exactly one call site of roll")]
fn crates_round_has_exactly_one_call_site_of_roll(world: &mut World) {
    let root = workspace_root();
    let round = root.join("crates/round");
    let mut files = Vec::new();
    files_under(&round, &mut files);
    let mut sites = 0usize;
    for path in &files {
        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let text = fs::read_to_string(path).unwrap();
        assert!(
            !text.contains("resolve_check("),
            "{} calls resolve_check",
            path.display()
        );
        sites += roll_calls(&text);
    }
    assert_eq!(sites, 1, "crates/round roll call sites");
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

#[when(expr = "a docks round is played")]
fn a_docks_round_is_played(world: &mut World) {
    let pack = copy_named("oneshot-docks", "stage-order");
    spookery_round::play_pack_once(&pack).unwrap_or_else(|err| panic!("play: {err}"));
    world.pack = Some(pack);
}

#[then(
    expr = "the round trace lists advance, complete, gather, resolve, apply, run_triggers, observe, record in that order"
)]
fn the_round_trace_lists_the_stages(world: &mut World) {
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

#[given(expr = "a round where Lark rolls grit")]
fn a_round_where_lark_rolls_grit(world: &mut World) {
    let clean = lark_pack();
    let played = copy_tree(&clean, "lark-alone");
    let (faces, total) = lark_roll(&played, false);
    let _ = fs::remove_dir_all(played);
    world.scratch = format!("{faces}|{total}");
    world.pack = Some(clean);
}

#[when(expr = "the same round is replayed with an extra unrelated NPC roll queued first")]
fn the_same_round_is_replayed_with_an_extra_roll(world: &mut World) {
    let pack = world.pack.as_ref().unwrap().clone();
    let (faces, total) = lark_roll(&pack, true);
    world.player_added = format!("{faces}|{total}");
}

#[then(expr = "Lark's faces and total are identical")]
fn larks_faces_and_total_are_identical(world: &mut World) {
    assert_eq!(world.player_added, world.scratch);
}

#[given(expr = "ten rolling intents in one round")]
fn ten_rolling_intents_in_one_round(world: &mut World) {
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
    world.now_bytes = spookery_kernel::store::codec::emit_save(&now)
        .unwrap()
        .into_bytes();
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
        spookery_round::from_journal(pack, None, None).unwrap_or_else(|err| panic!("{err:?}"));
    assert_eq!(
        report.now_yaml,
        fs::read(pack.join("state/save.yaml")).unwrap()
    );
}

#[given(expr = "a docks row overreach that adds {int} lamp oil and then removes {int}")]
fn a_docks_row_overreach(world: &mut World, add: i64, remove: i64) {
    let pack = copy_named("oneshot-docks", "rollback");
    append(
        &pack.join("rules/verbs.yaml"),
        &format!(
            "
  - id: overreach
    when:
      verb: overreach
    do:
      - item:
          who: place:docks_shop
          id: lamp_oil
          add: {add}
      - item:
          who: place:docks_shop
          id: lamp_oil
          add: -{remove}
"
        ),
    );
    stamp(&pack);
    quiet_mira(&pack);
    world.pack = Some(pack);
}

#[given(expr = "a docks row spilled that rolls grit and then removes {int} lamp oil")]
fn a_docks_row_spilled(world: &mut World, remove: i64) {
    let pack = copy_named("oneshot-docks", "spilled");
    append(
        &pack.join("rules/verbs.yaml"),
        &format!(
            "
  - id: spilled
    when:
      verb: spilled
    roll:
      check: grit
      target: 0
    do:
      - item:
          who: place:docks_shop
          id: lamp_oil
          add: -{remove}
"
        ),
    );
    stamp(&pack);
    quiet_mira(&pack);
    world.pack = Some(pack);
}

#[when(expr = "Mira plays {word}")]
fn mira_plays(world: &mut World, move_name: String) {
    let pack = world.pack.as_ref().unwrap().clone();
    let id = format!("{move_name}-1");
    let (now, rolls, tick) = play_queued(
        &pack,
        vec![queued(
            &id,
            "mira",
            ActorKind::Npc,
            &move_name,
            Target::Empty,
        )],
    );
    let oil = now
        .places
        .get("docks_shop")
        .and_then(|place| place.items.get("lamp_oil"))
        .copied()
        .map(i64::from)
        .unwrap_or(-1);
    world.now_bytes = oil.to_string().into_bytes();
    world.player_added = id;
    world.scratch = tick.to_string();
    world.mark = rolls
        .iter()
        .filter(|notice| match notice {
            RollReport::Rolled { actor_id, .. } => actor_id == "mira",
            RollReport::Refused { .. } => false,
        })
        .count();
}

#[then(expr = "lamp oil stays {int} and the round names {word}")]
fn lamp_oil_stays_and_the_round_names(world: &mut World, oil: i64, reason: String) {
    assert_refusal(world, oil, &reason);
}

#[then(expr = "lamp oil stays {int}, the round names {word}, and no roll is reported")]
fn lamp_oil_stays_the_round_names_and_no_roll_is_reported(
    world: &mut World,
    oil: i64,
    reason: String,
) {
    assert_refusal(world, oil, &reason);
    assert_eq!(world.mark, 0, "a refused move reported a roll");
}

#[given(expr = "two kernel waits queued for Mira")]
fn two_kernel_waits_queued_for_mira(world: &mut World) {
    world.pack = Some(copy_named("oneshot-docks", "second-wait"));
}

#[when(expr = "those waits are played")]
fn those_waits_are_played(world: &mut World) {
    let pack = world.pack.as_ref().unwrap().clone();
    let mut first = queued("wait-1", "mira", ActorKind::Npc, "wait", Target::Empty);
    let mut second = queued("wait-2", "mira", ActorKind::Npc, "wait", Target::Empty);
    first.source = Source::Kernel;
    second.source = Source::Kernel;
    let (now, _rolls, tick) = play_queued(&pack, vec![first, second]);
    world.scratch = tick.to_string();
    world.now_bytes = spookery_kernel::store::codec::emit_save(&now)
        .unwrap()
        .into_bytes();
}

#[then(expr = "only the first wait lands and the second is INELIGIBLE")]
fn only_the_first_wait_lands(world: &mut World) {
    let tick: Value = serde_json::from_str(&world.scratch).unwrap();
    let applied: Vec<&str> = tick["intents"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["id"].as_str())
        .collect();
    assert!(applied.contains(&"wait-1"), "{tick}");
    assert!(!applied.contains(&"wait-2"), "{tick}");
    let rejected = tick["rejected"].as_array().unwrap();
    assert!(
        rejected
            .iter()
            .any(|item| item["id"] == "wait-2" && item["reason"] == "INELIGIBLE"),
        "{tick}"
    );
    let now =
        spookery_kernel::store::codec::load_save(std::str::from_utf8(&world.now_bytes).unwrap())
            .unwrap();
    assert_eq!(
        now.actors
            .get("mira")
            .map(|actor| actor.act().last_act_tick),
        Some(1)
    );
}

#[when(expr = "Lark and Briar strike each other")]
fn lark_and_briar_strike_each_other(world: &mut World) {
    let pack = copy_named("sample-d20", "mutual-strike");
    let (_now, rolls, _tick) = play_queued(
        &pack,
        vec![
            queued(
                "lark-strike",
                "pc_lark",
                ActorKind::Player,
                "strike",
                Target::Actor("npc_briar".to_string()),
            ),
            queued(
                "briar-strike",
                "npc_briar",
                ActorKind::Npc,
                "strike",
                Target::Actor("pc_lark".to_string()),
            ),
        ],
    );
    world.pack = Some(pack);
    world.scratch = rolls
        .iter()
        .filter_map(|notice| match notice {
            RollReport::Rolled { actor_id, out } => Some(format!(
                "{actor_id}={}:{}:{}",
                out.faces
                    .iter()
                    .map(|face| face.to_string())
                    .collect::<Vec<_>>()
                    .join(","),
                out.total,
                out.target
            )),
            RollReport::Refused { .. } => None,
        })
        .collect::<Vec<_>>()
        .join(";");
}

#[then(expr = "each strike is scored on the other's own roll")]
fn each_strike_is_scored_on_the_others_own_roll(world: &mut World) {
    let pack = world.pack.as_ref().unwrap();
    let lark = strike_line(&world.scratch, "pc_lark");
    let briar = strike_line(&world.scratch, "npc_briar");
    let lark_once = indexed_roll(pack, "pc_lark", 0);
    let lark_again = indexed_roll(pack, "pc_lark", 1);
    let briar_once = indexed_roll(pack, "npc_briar", 0);
    let briar_again = indexed_roll(pack, "npc_briar", 1);
    assert_ne!(lark_once.1, lark_again.1, "lark's two indexes collide");
    assert_ne!(briar_once.1, briar_again.1, "briar's two indexes collide");
    assert_eq!(lark.0, lark_once.0, "lark was rolled twice");
    assert_eq!(briar.0, briar_once.0, "briar was rolled twice");
    assert_eq!(lark.1, lark_once.1);
    assert_eq!(briar.1, briar_once.1);
    assert_eq!(lark.2, briar.1, "lark's target is not briar's roll");
    assert_eq!(briar.2, lark.1, "briar's target is not lark's roll");
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
        &pack.join("rules/verbs.yaml"),
        "
  - id: two_stock
    when:
      verb: two_stock
    do:
      - item:
          who: place:docks_shop
          id: lamp_oil
          add: 1
      - item:
          who: place:docks_shop
          id: lamp_oil
          add: 1
",
    );
    stamp(&pack);
    quiet_mira(&pack);
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
            .and_then(|place| place.items.get("lamp_oil"))
            .copied()
            .map(i64::from)
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

fn roll_calls(text: &str) -> usize {
    let bytes = text.as_bytes();
    let mut count = 0usize;
    let mut from = 0usize;
    while let Some(at) = text[from..].find("roll(") {
        let index = from + at;
        let boundary =
            index == 0 || (!bytes[index - 1].is_ascii_alphanumeric() && bytes[index - 1] != b'_');
        if boundary {
            count += 1;
        }
        from = index + 5;
    }
    count
}

fn assert_refusal(world: &World, oil: i64, reason: &str) {
    assert_eq!(String::from_utf8_lossy(&world.now_bytes), oil.to_string());
    let tick: Value = serde_json::from_str(&world.scratch).unwrap();
    assert!(tick["effects"].as_array().unwrap().is_empty(), "{tick}");
    let rejected = tick["rejected"].as_array().unwrap();
    assert!(
        rejected
            .iter()
            .any(|item| item["id"] == world.player_added && item["reason"] == reason),
        "{tick}"
    );
}

fn strike_line(scratch: &str, actor: &str) -> (String, i64, i64) {
    let prefix = format!("{actor}=");
    let line = scratch
        .split(';')
        .find(|item| item.starts_with(&prefix))
        .unwrap_or_else(|| panic!("no roll for {actor} in {scratch}"));
    let rest = line.trim_start_matches(&prefix);
    let mut parts = rest.split(':');
    let faces = parts.next().unwrap().to_string();
    let total = parts.next().unwrap().parse().unwrap();
    let target = parts.next().unwrap().parse().unwrap();
    (faces, total, target)
}

fn indexed_roll(pack: &Path, actor_id: &str, roll_index: u32) -> (String, i64) {
    let world = spookery_kernel::World::open(pack).unwrap_or_else(|err| panic!("open: {err}"));
    let actor = world
        .now()
        .actors
        .get(actor_id)
        .unwrap_or_else(|| panic!("missing {actor_id}"))
        .clone();
    let kind = if actor.is_pc() {
        ActorKind::Player
    } else {
        ActorKind::Npc
    };
    let viewer = ViewerId::from_kind(kind, actor_id).unwrap();
    let check = world.canon().checks.get("strike").unwrap().clone();
    let out = roll(
        RollInput {
            check: &check,
            defs: &world.canon().meters,
            actor_id,
            actor: &actor,
            viewer: &viewer,
            target: 0,
            partial_at: None,
        },
        &RollKey {
            world_id: world.now().shard_id.clone(),
            round_index: world.now().tick_index,
            rule_id: "strike".to_string(),
            subject_id: actor_id.to_string(),
            roll_index,
        },
    )
    .unwrap_or_else(|err| panic!("roll: {err}"));
    let faces = out
        .faces
        .iter()
        .map(|face| face.to_string())
        .collect::<Vec<_>>()
        .join(",");
    (faces, out.total)
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
        &pack.join("rules/verbs.yaml"),
        "
  - id: grit
    when:
      verb: grit
    roll:
      check: grit
      target: 0
    do: []
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
    spookery_round::play(&mut world, RoundCause::Manual, &wall)
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

fn quiet_mira(pack: &Path) {
    let path = pack.join("cast/mira.md");
    let text = fs::read_to_string(&path).unwrap();
    let next = text.replace("  eligible: true\n", "  eligible: false\n");
    assert_ne!(text, next, "mira was already ineligible");
    fs::write(&path, next).unwrap();
    stamp(pack);
}

fn stamp(pack: &Path) {
    let now_path = pack.join("state/save.yaml");
    let mut now =
        spookery_kernel::store::codec::load_save(&fs::read_to_string(&now_path).unwrap()).unwrap();
    now.pack_rev = spookery_kernel::canon::pack_rev(pack).unwrap();
    fs::write(
        &now_path,
        spookery_kernel::store::codec::emit_save(&now).unwrap(),
    )
    .unwrap();
}

fn append(path: &Path, extra: &str) {
    let mut text = fs::read_to_string(path).unwrap();
    text.push_str(extra);
    fs::write(path, text).unwrap();
}

fn tick_value(pack: &Path, name: &str) -> Value {
    let text = fs::read_to_string(pack.join("state/journal").join(name))
        .unwrap_or_else(|err| panic!("{err}"));
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
