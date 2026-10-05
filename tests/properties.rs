use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use spookery::seat::{
    commit_queued, gm_tool, on_scheduled_wake, queue_wait, release_player_seat, seat_player,
    turn, SocketEnd, Turn,
};
use spookery_kernel::apply::apply_effect;
use spookery_kernel::check::CheckNotice;
use spookery_kernel::codec::emit_now;
use spookery_kernel::effect::{Effect, EffectClass};
use spookery_kernel::model::{
    ActorKind, ActorLive, Intent, IntentArgs, Now, PauseView, PlaceLive, Presence, RunState,
    Source, Target, TickCounters, ViewerId,
};
use spookery_kernel::shard::{Shard, Txn};
use spookery_kernel::Reason;
use spookery_tick::{replay_recorded, run_catchup, run_tick, tick_pack, Catchup};

const LEDGER: &str = "The lamp-oil ledger is short.";

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap_or_else(|err| panic!("mkdir {}: {err}", to.display()));
    for entry in fs::read_dir(from).unwrap_or_else(|err| panic!("read {}: {err}", from.display())) {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &dest);
        } else {
            fs::copy(entry.path(), &dest)
                .unwrap_or_else(|err| panic!("copy {}: {err}", entry.path().display()));
        }
    }
}

fn scratch(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "spook-prop-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ))
}

fn temp_pack(label: &str) -> PathBuf {
    let pack = scratch(label);
    let _ = fs::remove_dir_all(&pack);
    copy_dir(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("packs/oneshot-docks"),
        &pack,
    );
    pack
}

fn open(pack: &Path) -> Shard {
    spookery_kernel::open_shard(pack).unwrap_or_else(|err| panic!("open_shard: {err}"))
}

fn now_of(pack: &Path) -> Now {
    open(pack).now().clone()
}

fn write_text(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .unwrap_or_else(|err| panic!("mkdir {}: {err}", parent.display()));
    }
    fs::write(path, text).unwrap_or_else(|err| panic!("write {}: {err}", path.display()));
}

fn stamp_pack_rev(pack: &Path) {
    let now_path = pack.join("state/now.yaml");
    let mut now = spookery_kernel::codec::load_now(&fs::read_to_string(&now_path).unwrap()).unwrap();
    now.pack_rev = spookery_kernel::canon::pack_rev(pack).unwrap();
    fs::write(&now_path, emit_now(&now).unwrap()).unwrap();
}

fn lamp_oil(now: &Now) -> i64 {
    now.places
        .get("docks_shop")
        .and_then(|place| place.stock.get("lamp_oil"))
        .copied()
        .unwrap_or_else(|| panic!("lamp_oil missing"))
}

fn flags_of(now: &Now, id: &str) -> Vec<String> {
    match now.actors.get(id) {
        Some(ActorLive::Npc { flags, .. } | ActorLive::Pc { flags, .. }) => flags.clone(),
        None => panic!("{id} missing"),
    }
}

fn tick_value(pack: &Path, name: &str) -> Value {
    let text = fs::read_to_string(pack.join("state/log").join(name))
        .unwrap_or_else(|err| panic!("read {name}: {err}"));
    serde_json::from_str(text.trim_end()).unwrap_or_else(|err| panic!("json {name}: {err}"))
}

fn tick_names(pack: &Path) -> Vec<String> {
    let dir = pack.join("state/log");
    let mut names = Vec::new();
    if dir.is_dir() {
        for entry in fs::read_dir(&dir).unwrap() {
            let name = entry.unwrap().file_name().to_string_lossy().into_owned();
            if name.starts_with("tick-") && name.ends_with(".jsonl") {
                names.push(name);
            }
        }
    }
    names.sort();
    names
}

fn snapshot(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
        let mut entries: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap())
            .collect();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, root, out);
            } else {
                let rel = path.strip_prefix(root).unwrap().to_path_buf();
                out.push((rel, fs::read(&path).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out
}

#[test]
fn closed_secret() {
    let pack = temp_pack("secret");
    let mira = fs::read_to_string(pack.join("cast/mira.md")).unwrap();
    let aldric = fs::read_to_string(pack.join("cast/aldric.md")).unwrap();
    assert!(mira.contains(LEDGER), "fixture secret is on mira");
    assert!(!aldric.contains(LEDGER), "fixture secret is on aldric");
    for tick in 1..=100 {
        tick_pack(&pack).unwrap_or_else(|err| panic!("tick {tick}: {err}"));
    }
    let perceive = {
        let shard = open(&pack);
        format!(
            "{:?}",
            spookery_kernel::perceive(&shard, "npc:aldric").unwrap()
        )
    };
    let memories = fs::read_to_string(pack.join("state/memories/aldric.jsonl")).unwrap_or_default();
    let rumors = format!(
        "{}\n{}",
        fs::read_to_string(pack.join("state/rumors.yaml")).unwrap_or_default(),
        fs::read_to_string(pack.join("state/now.yaml")).unwrap_or_default()
    );
    assert!(!perceive.contains(LEDGER), "ledger sentence in Aldric perceive");
    assert!(!memories.contains(LEDGER), "ledger sentence in Aldric memories");
    assert!(!rumors.contains(LEDGER), "ledger sentence in rumors");
    let _ = fs::remove_dir_all(&pack);
}

#[test]
fn no_self_win() {
    let pack = temp_pack("self-win");
    for tick in 1..=12 {
        tick_pack(&pack).unwrap_or_else(|err| panic!("tick {tick}: {err}"));
    }
    let now = now_of(&pack);
    assert_eq!(now.tick_index, 12, "twelve empty-table ticks commit");
    let clock = now
        .clocks
        .get("assassination_debt")
        .unwrap_or_else(|| panic!("assassination_debt missing"));
    assert_eq!(clock.value, 2);
    assert_eq!(clock.max, 6);
    assert!(clock.paused);
    assert!(clock.requires_presence);
    assert_eq!(clock.owners, vec!["pc_rowan".to_string()]);
    let _ = fs::remove_dir_all(&pack);

    let seated = temp_pack("agenda-line");
    let mut shard = open(&seated);
    seat_player(&mut shard, "pc_rowan").unwrap();
    assert_eq!(shard.now().clocks["assassination_debt"].value, 2);
    for line in ["advance_clock", "season_advance"] {
        let err = match turn(&mut shard, "pc_rowan", line, true) {
            Err(err) => err,
            Ok(_) => panic!("{line} was queued"),
        };
        assert_eq!(err, Reason::NotInSpec, "{line}");
        let args = json!({
            "actor_id": "pc_rowan",
            "actor_kind": "player",
            "source": "player",
            "move": line,
            "target": {},
            "args": {}
        });
        let tool = gm_tool(
            &mut shard,
            "intent",
            Some(args.as_object().unwrap().clone()),
        );
        assert_eq!(tool.value["code"], "NOT_IN_SPEC", "{line}");
    }
    assert!(shard.queued_intents().is_empty());
    assert_eq!(shard.now().clocks["assassination_debt"].value, 2);
    assert!(!seated.join("state/proposals/season_advance.yaml").exists());
    let _ = fs::remove_dir_all(&seated);
}

#[test]
fn shop_lives() {
    let pack = temp_pack("shop");
    {
        let shard = open(&pack);
        let mut now = shard.now().clone();
        drop(shard);
        *now.actors
            .get_mut("mira")
            .unwrap()
            .place_mut() = "docks_shop".to_string();
        *now.actors
            .get_mut("pc_rowan")
            .unwrap()
            .place_mut() = "docks_street".to_string();
        now.presence.humans = 0;
        now.presence.actors_present.clear();
        fs::write(pack.join("state/now.yaml"), emit_now(&now).unwrap()).unwrap();
    }
    let rate = open(&pack)
        .canon()
        .profile("mira")
        .and_then(|profile| profile.stock_rate.get("docks_shop"))
        .and_then(|rates| rates.get("lamp_oil"))
        .copied()
        .unwrap();
    assert_eq!(rate, 4);
    assert_eq!(lamp_oil(&now_of(&pack)), 4);
    assert_eq!(now_of(&pack).actors.get("mira").unwrap().place(), "docks_shop");
    assert_eq!(
        now_of(&pack).actors.get("pc_rowan").unwrap().place(),
        "docks_street"
    );
    tick_pack(&pack).unwrap_or_else(|err| panic!("tick: {err}"));
    let now = now_of(&pack);
    assert_eq!(lamp_oil(&now), 8, "mira restocks lamp_oil by the docks_shop rate");
    assert_eq!(now.actors.get("mira").unwrap().place(), "docks_shop");
    assert_ne!(now.actors.get("pc_rowan").unwrap().place(), "docks_shop");
    let _ = fs::remove_dir_all(&pack);
}

const SEASON: &str = "\
schema: proposal/v0
id: season_advance
kind: other
target: \"\"
summary: season_advance
actor_id: pc_rowan
source: system
tick_index: 12
";

#[test]
fn catchup_cap() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let pack = scratch("catchup");
    let _ = fs::remove_dir_all(&pack);
    let rules = manifest.join("packs/oneshot-docks/rules");
    write_text(
        &pack.join("pack.yaml"),
        "\
schema: pack/v0
id: ex
title: Ex
system: ex
start:
  place: gate
  pc: pc_rowan
",
    );
    write_text(
        &pack.join("world/places/gate.md"),
        "\
---
schema: place/v0
id: gate
facts:
  - id: fact_gate
    text: The gate is shut.
    hidden: false
---
A gate.
",
    );
    write_text(
        &pack.join("cast/pc_rowan.md"),
        "\
---
schema: cast/v0
id: pc_rowan
kind: pc_template
facts: []
---
Rowan.
",
    );
    for name in ["moves.yaml", "checks.yaml", "tick.yaml"] {
        fs::copy(rules.join(name), {
            let dest = pack.join("rules").join(name);
            fs::create_dir_all(dest.parent().unwrap()).unwrap();
            dest
        })
        .unwrap_or_else(|err| panic!("copy {name}: {err}"));
    }
    write_text(
        &pack.join("state/actors/pc_rowan.yaml"),
        "\
schema: sheet/v0
id: pc_rowan
template: cast/pc_rowan.md
stats:
  grit: 1
resources: {}
inventory: []
flags: []
",
    );
    let rev = spookery_kernel::canon::pack_rev(&pack).unwrap();
    let mut places = BTreeMap::new();
    places.insert(
        "gate".to_string(),
        PlaceLive {
            stock: BTreeMap::new(),
            hours: BTreeMap::new(),
            notes: Vec::new(),
            actors: vec!["pc_rowan".to_string()],
        },
    );
    let mut actors = BTreeMap::new();
    actors.insert(
        "pc_rowan".to_string(),
        ActorLive::Pc {
            place: "gate".to_string(),
            flags: Vec::new(),
            busy: false,
            known_facts: Vec::new(),
            sheet: "state/actors/pc_rowan.yaml".to_string(),
        },
    );
    let now = Now {
        schema: "now/v0".to_string(),
        shard_id: "ex".to_string(),
        pack_rev: rev,
        state_rev: 0,
        tick_index: 0,
        world_time: 0,
        last_tick_at: "2026-01-01T00:00:00Z".to_string(),
        presence: Presence {
            humans: 0,
            actors_present: Vec::new(),
        },
        pause: PauseView {
            named_plot_clocks: RunState::Paused,
            relationship_drift: RunState::Running,
            subsistence_clocks: RunState::Running,
            travel: RunState::Running,
        },
        places,
        actors,
        clocks: BTreeMap::new(),
        rumors: Vec::new(),
        messages: Vec::new(),
        proposals: Vec::new(),
    };
    write_text(&pack.join("state/now.yaml"), &emit_now(&now).unwrap());
    let place_before = fs::read(pack.join("world/places/gate.md")).unwrap();
    let cast_before = fs::read(pack.join("cast/pc_rowan.md")).unwrap();
    let mut shard = open(&pack);
    let wall = shard.now().last_tick_at.clone();
    run_catchup(&mut shard, &wall, 72).unwrap_or_else(|err| panic!("catch-up: {err}"));
    let saved = open(&pack);
    let now = saved.now();
    assert_eq!(now.state_rev, 12, "twelve commits");
    assert_eq!(now.tick_index, 12);
    assert_eq!(now.world_time, 720);
    assert_eq!(now.last_tick_at, "2026-01-01T00:00:00Z");
    assert_eq!(now.proposals, vec!["season_advance".to_string()]);
    assert_eq!(tick_names(&pack).len(), 12);
    let proposal = fs::read_to_string(pack.join("state/proposals/season_advance.yaml")).unwrap();
    assert_eq!(proposal, SEASON);
    assert_eq!(fs::read(pack.join("world/places/gate.md")).unwrap(), place_before);
    assert_eq!(fs::read(pack.join("cast/pc_rowan.md")).unwrap(), cast_before);
    let _ = fs::remove_dir_all(&pack);
}

#[test]
fn busy_skip() {
    let pack = temp_pack("busy");
    {
        let shard = open(&pack);
        let mut now = shard.now().clone();
        drop(shard);
        now.presence.humans = 1;
        now.presence.actors_present = vec!["pc_rowan".to_string()];
        now.world_time = 360;
        *now.actors.get_mut("mira").unwrap().place_mut() = "docks_shop".to_string();
        fs::write(pack.join("state/now.yaml"), emit_now(&now).unwrap()).unwrap();
    }
    let mut shard = open(&pack);
    let wall = shard.now().last_tick_at.clone();
    run_tick(
        &mut shard,
        spookery_tick::Reason::Manual,
        &wall,
        Catchup::none(),
    )
    .unwrap();
    let now = shard.now();
    assert_eq!(now.world_time, 365);
    assert_eq!(now.actors.get("mira").unwrap().place(), "docks_shop");
    assert_eq!(
        now.actors.get("mira").unwrap().tick().unwrap().last_act_tick,
        0
    );
    assert_eq!(lamp_oil(now), 4);
    assert_eq!(now.actors.get("bram").unwrap().place(), "docks_shop");
    assert_eq!(
        now.actors.get("bram").unwrap().tick().unwrap().last_act_tick,
        1
    );
    let _ = fs::remove_dir_all(&pack);
}

fn mira_intent() -> Intent {
    Intent {
        id: "drop-txn".to_string(),
        tick_index: 1,
        actor_id: "mira".to_string(),
        actor_kind: ActorKind::Npc,
        source: Source::Kernel,
        move_name: "stock".to_string(),
        target: Target::Place("docks_shop".to_string()),
        args: IntentArgs::default(),
        rationale_code: String::new(),
    }
}

fn stock_effect(delta: i64) -> Effect {
    Effect {
        class: EffectClass::Stock {
            place: "docks_shop".to_string(),
            sku: "lamp_oil".to_string(),
            delta,
        },
        as_viewer: ViewerId::Npc("mira".to_string()),
        intent_id: "drop-txn".to_string(),
    }
}

#[test]
fn atomic_commit() {
    let dropped = temp_pack("drop");
    let now_path = dropped.join("state/now.yaml");
    let before = fs::read(&now_path).unwrap();
    let shard = open(&dropped);
    let state_rev = shard.now().state_rev;
    let proposals = shard.now().proposals.clone();
    let oil = lamp_oil(shard.now());
    let delta = if oil > 0 { -1 } else { 1 };
    let mut txn = Txn::begin(&shard);
    apply_effect(&mut txn, &mira_intent(), stock_effect(delta)).unwrap();
    assert_eq!(lamp_oil(txn.now()), oil + delta);
    drop(txn);
    drop(shard);
    assert!(!dropped.join("state/.txn").exists());
    assert_eq!(fs::read(&now_path).unwrap(), before);
    let restarted = open(&dropped);
    assert_eq!(restarted.now().state_rev, state_rev);
    assert_eq!(lamp_oil(restarted.now()), oil);
    assert_eq!(restarted.now().proposals, proposals);
    assert!(!dropped.join("state/proposals").exists());
    assert!(!dropped.join("state/memories").exists());
    let _ = fs::remove_dir_all(&dropped);

    let incomplete = temp_pack("incomplete");
    let now_path = incomplete.join("state/now.yaml");
    let before = fs::read(&now_path).unwrap();
    let txn = incomplete.join("state/.txn");
    fs::create_dir_all(txn.join("prev")).unwrap();
    fs::write(txn.join("prev/now.yaml"), b"not-yaml\n").unwrap();
    let opened = open(&incomplete);
    assert_eq!(fs::read(&now_path).unwrap(), before);
    assert_eq!(lamp_oil(opened.now()), lamp_oil(&spookery_kernel::codec::load_now(std::str::from_utf8(&before).unwrap()).unwrap()));
    assert!(!txn.exists());
    let _ = fs::remove_dir_all(&incomplete);

    let same = temp_pack("same-rev");
    let now_path = same.join("state/now.yaml");
    let original = fs::read(&now_path).unwrap();
    let shard = open(&same);
    let mut dirty = shard.now().clone();
    drop(shard);
    dirty
        .places
        .get_mut("docks_shop")
        .unwrap()
        .stock
        .insert("lamp_oil".to_string(), 999);
    fs::write(&now_path, emit_now(&dirty).unwrap()).unwrap();
    let txn = same.join("state/.txn");
    fs::create_dir_all(txn.join("prev")).unwrap();
    fs::write(txn.join("prev/now.yaml"), &original).unwrap();
    fs::write(txn.join("COMPLETE"), b"").unwrap();
    let opened = open(&same);
    assert_eq!(fs::read(&now_path).unwrap(), original);
    assert_eq!(
        lamp_oil(opened.now()),
        lamp_oil(&spookery_kernel::codec::load_now(std::str::from_utf8(&original).unwrap()).unwrap())
    );
    assert!(!txn.exists());
    let _ = fs::remove_dir_all(&same);

    let next = temp_pack("next-rev");
    let shard = open(&next);
    let prev = shard.now().clone();
    let mut live = prev.clone();
    drop(shard);
    live.state_rev = prev.state_rev.saturating_add(1);
    let kept_oil = lamp_oil(&prev).saturating_add(1);
    live.places
        .get_mut("docks_shop")
        .unwrap()
        .stock
        .insert("lamp_oil".to_string(), kept_oil);
    fs::write(next.join("state/now.yaml"), emit_now(&live).unwrap()).unwrap();
    let txn = next.join("state/.txn");
    fs::create_dir_all(txn.join("prev")).unwrap();
    fs::write(txn.join("prev/now.yaml"), emit_now(&prev).unwrap()).unwrap();
    fs::write(txn.join("COMPLETE"), b"").unwrap();
    let log = next.join("state/log");
    fs::create_dir_all(&log).unwrap();
    fs::write(
        log.join("tick-0001.jsonl"),
        format!("{{\"tick_index\":1,\"state_rev_after\":{}}}\n", live.state_rev),
    )
    .unwrap();
    fs::write(
        log.join("tick-0099.jsonl"),
        format!(
            "{{\"tick_index\":99,\"state_rev_after\":{}}}\n",
            live.state_rev.saturating_add(5)
        ),
    )
    .unwrap();
    let memories = next.join("state/memories");
    fs::create_dir_all(&memories).unwrap();
    fs::write(
        memories.join("mira.jsonl"),
        "{\"tick_index\":1,\"text\":\"kept-line\"}\n{\"tick_index\":99,\"text\":\"orphan-line\"}\n",
    )
    .unwrap();
    let proposals = next.join("state/proposals");
    fs::create_dir_all(&proposals).unwrap();
    fs::write(proposals.join("orphan.yaml"), b"orphan\n").unwrap();
    let opened = open(&next);
    assert_eq!(opened.now().state_rev, live.state_rev);
    assert_eq!(lamp_oil(opened.now()), kept_oil);
    assert!(!txn.exists());
    assert!(log.join("tick-0001.jsonl").is_file());
    assert!(!log.join("tick-0099.jsonl").exists());
    let memory = fs::read_to_string(memories.join("mira.jsonl")).unwrap();
    assert!(memory.contains("kept-line"));
    assert!(!memory.contains("orphan-line"));
    assert!(!proposals.join("orphan.yaml").exists());
    let _ = fs::remove_dir_all(&next);
}

#[test]
fn replay_byte_compares_now() {
    let stamp = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let start = std::env::temp_dir().join(format!("spook-prop-replay-start-{stamp}"));
    let played = std::env::temp_dir().join(format!("spook-prop-replay-played-{stamp}"));
    let _ = fs::remove_dir_all(&start);
    let _ = fs::remove_dir_all(&played);
    copy_dir(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("packs/oneshot-docks"),
        &start,
    );
    copy_dir(&start, &played);
    for tick in 1..=20 {
        tick_pack(&played).unwrap_or_else(|err| panic!("tick {tick}: {err}"));
    }
    let golden = fs::read(played.join("state/now.yaml")).unwrap();
    let source_log = fs::read(played.join("state/log/tick-0020.jsonl")).unwrap();
    let temp = replay_recorded(&played, &start).unwrap();
    assert_eq!(fs::read(temp.join("state/now.yaml")).unwrap(), golden);
    assert_ne!(temp, played);
    assert_eq!(fs::read(played.join("state/now.yaml")).unwrap(), golden);
    assert_eq!(fs::read(played.join("state/log/tick-0020.jsonl")).unwrap(), source_log);
    let _ = fs::remove_dir_all(&start);
    let _ = fs::remove_dir_all(&played);
    let _ = fs::remove_dir_all(&temp);
}

#[test]
fn budget_caps() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let pack = scratch("budget");
    let _ = fs::remove_dir_all(&pack);
    let rules = manifest.join("packs/oneshot-docks/rules");
    write_text(
        &pack.join("pack.yaml"),
        "\
schema: pack/v0
id: crowd
title: Crowd
system: crowd
start:
  place: yard
  pc: n1
",
    );
    write_text(
        &pack.join("world/places/yard.md"),
        "\
---
schema: place/v0
id: yard
facts:
  - id: fact_yard
    text: The yard is open.
    hidden: false
---
A yard.
",
    );
    let mut actors = BTreeMap::new();
    for index in 1..=50 {
        let id = format!("n{index}");
        write_text(
            &pack.join(format!("cast/{id}.md")),
            &format!(
                "\
---
schema: cast/v0
id: {id}
kind: npc
facts: []
tick:
  eligible: true
  tier: minor
  plot_relevant: false
  range:
    - yard
  schedule:
    - start: 0
      end: 24
      place: yard
      activity: work
  agenda:
    - row: work
      pause: subsistence_clocks
      schedule: here
      activity: work
---
{id}
"
            ),
        );
        actors.insert(
            id,
            ActorLive::Npc {
                place: "yard".to_string(),
                flags: Vec::new(),
                busy: false,
                needs: BTreeMap::new(),
                resources: BTreeMap::new(),
                relationships: BTreeMap::new(),
                reputation: BTreeMap::new(),
                known_facts: Vec::new(),
                tick: TickCounters {
                    last_act_tick: 0,
                    idle_acts_today: 0,
                    live_acts_this_hour: 0,
                },
            },
        );
    }
    for name in ["moves.yaml", "checks.yaml", "tick.yaml"] {
        let dest = pack.join("rules").join(name);
        fs::create_dir_all(dest.parent().unwrap()).unwrap();
        fs::copy(rules.join(name), &dest).unwrap();
    }
    let rev = spookery_kernel::canon::pack_rev(&pack).unwrap();
    let mut places = BTreeMap::new();
    places.insert(
        "yard".to_string(),
        PlaceLive {
            stock: BTreeMap::new(),
            hours: BTreeMap::new(),
            notes: Vec::new(),
            actors: Vec::new(),
        },
    );
    let now = Now {
        schema: "now/v0".to_string(),
        shard_id: "crowd".to_string(),
        pack_rev: rev,
        state_rev: 0,
        tick_index: 0,
        world_time: 0,
        last_tick_at: "2026-01-01T00:00:00Z".to_string(),
        presence: Presence {
            humans: 0,
            actors_present: Vec::new(),
        },
        pause: PauseView {
            named_plot_clocks: RunState::Paused,
            relationship_drift: RunState::Running,
            subsistence_clocks: RunState::Running,
            travel: RunState::Running,
        },
        places,
        actors,
        clocks: BTreeMap::new(),
        rumors: Vec::new(),
        messages: Vec::new(),
        proposals: Vec::new(),
    };
    write_text(&pack.join("state/now.yaml"), &emit_now(&now).unwrap());
    tick_pack(&pack).unwrap_or_else(|err| panic!("tick: {err}"));
    let now = now_of(&pack);
    assert_eq!(now.tick_index, 1);
    assert_eq!(now.actors.values().filter(|actor| actor.is_npc()).count(), 50);
    let acted = now
        .actors
        .values()
        .filter(|actor| actor.tick().is_some_and(|tick| tick.last_act_tick == 1))
        .count();
    let applied = tick_value(&pack, "tick-0001.jsonl")["intents"]
        .as_array()
        .unwrap()
        .len();
    assert!(acted >= 1 && acted <= 8, "acted {acted}");
    assert!(applied >= 1 && applied <= 12, "applied {applied}");
    let _ = fs::remove_dir_all(&pack);
}

#[test]
fn tool_scope() {
    let pack = temp_pack("scope");
    let before = snapshot(&pack);
    let shard = open(&pack);
    let call = |as_viewer: Option<&str>, focus: Option<&str>, budget: Option<u64>| {
        spookery_mcp::tick_tools::perceive(
            &shard,
            &spookery_mcp::tick_tools::PerceiveIn {
                as_viewer: as_viewer.map(str::to_string),
                focus: focus.map(str::to_string),
                budget_tokens: budget,
            },
        )
    };
    assert_eq!(call(None, None, None).unwrap_err().token(), "UNSCOPED_PERCEIVE");
    for as_viewer in ["", "gm", "system", "system:clock", "npc:", "player:pc_rowan"] {
        assert_eq!(
            call(Some(as_viewer), None, None).unwrap_err().token(),
            "UNSCOPED_PERCEIVE",
            "{as_viewer}"
        );
    }
    assert_eq!(
        call(Some("npc:mira"), Some("stock"), None).unwrap_err().token(),
        "SCHEMA"
    );
    let kernel = spookery_kernel::perceive(&shard, "npc:mira").unwrap();
    for focus in [None, Some("place"), Some("actor")] {
        assert_eq!(call(Some("npc:mira"), focus, None).unwrap(), kernel);
    }
    assert_eq!(call(Some("npc:mira"), None, Some(800)).unwrap(), kernel);
    let capped = call(Some("npc:mira"), None, Some(0)).unwrap();
    assert!(kernel.known_facts.iter().any(|fact| !fact.body.is_empty()));
    assert_eq!(capped.known_facts.len(), kernel.known_facts.len());
    assert!(capped.known_facts.iter().all(|fact| fact.body.is_empty()));
    let ledger = capped
        .known_facts
        .iter()
        .find(|fact| fact.id == "fact_ledger_short")
        .unwrap();
    assert_eq!(ledger.text, LEDGER);
    assert!(!capped.unknown_mentioned.iter().any(|id| id == "fact_ledger_short"));
    assert!(capped.unknown_mentioned.iter().any(|id| id == "fact_room_lamp"));
    drop(shard);
    assert_eq!(snapshot(&pack), before);
    let _ = fs::remove_dir_all(&pack);
}

#[test]
fn proposal_only() {
    let pack = temp_pack("proposal");
    let moves = pack.join("rules/moves.yaml");
    let mut moves_text = fs::read_to_string(&moves).unwrap();
    moves_text.push_str(
        "\n  - name: file_proposal\n    audience: player\n    token: none\n    on: speaker\n    effects:\n      - class: proposal\n        proposal: delete_docks_shop\n        kind: delete_place\n        target: docks_shop\n        summary: delete place\n",
    );
    fs::write(&moves, moves_text).unwrap();
    stamp_pack_rev(&pack);
    let place_path = pack.join("world/places/docks_shop.md");
    let place_before = fs::read(&place_path).unwrap();
    let mut shard = open(&pack);
    assert_eq!(shard.now().presence.humans, 0);
    let wall = shard.now().last_tick_at.clone();
    shard.queue_intent(Intent {
        id: "idle-delete-place".to_string(),
        tick_index: shard.now().tick_index + 1,
        actor_id: "mira".to_string(),
        actor_kind: ActorKind::Npc,
        source: Source::Kernel,
        move_name: "file_proposal".to_string(),
        target: Target::Empty,
        args: IntentArgs::default(),
        rationale_code: String::new(),
    });
    run_tick(
        &mut shard,
        spookery_tick::Reason::Manual,
        &wall,
        Catchup::none(),
    )
    .unwrap();
    assert!(pack.join("state/proposals/delete_docks_shop.yaml").is_file());
    assert_eq!(fs::read(&place_path).unwrap(), place_before);
    let _ = fs::remove_dir_all(&pack);
}

fn markdown_files(dir: &Path, out: &mut Vec<PathBuf>) {
    if !dir.exists() {
        return;
    }
    let mut entries: Vec<_> = fs::read_dir(dir).unwrap().map(|entry| entry.unwrap()).collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            markdown_files(&path, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("md") {
            out.push(path);
        }
    }
}

#[test]
fn strip_bodies_and_hidden_string_lint() {
    let pack = temp_pack("strip");
    let mut files = Vec::new();
    markdown_files(&pack.join("cast"), &mut files);
    markdown_files(&pack.join("world"), &mut files);
    assert!(!files.is_empty());
    for path in &files {
        let text = fs::read_to_string(path).unwrap();
        let stripped = spookery_kernel::lint::strip_body(&text).unwrap();
        assert!(!stripped.contains("Mira keeps the shop"), "{}", path.display());
        assert!(!stripped.contains("Bram walks"), "{}", path.display());
        fs::write(path, stripped).unwrap();
    }
    let mira = fs::read_to_string(pack.join("cast/mira.md")).unwrap();
    assert!(mira.contains(LEDGER), "frontmatter secret removed");
    assert!(!fs::read_to_string(pack.join("cast/aldric.md")).unwrap().contains(LEDGER));
    stamp_pack_rev(&pack);
    open(&pack);
    tick_pack(&pack).unwrap_or_else(|err| panic!("stripped pack did not tick: {err}"));
    let perceive = format!("{:?}", spookery_kernel::perceive(&open(&pack), "npc:aldric").unwrap());
    assert!(!perceive.contains(LEDGER));

    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("packs/oneshot-docks");
    assert!(spookery_kernel::lint::lint(&fixture).unwrap().is_empty());
    let owner = temp_pack("lint-owner");
    let mut body = fs::read_to_string(owner.join("cast/mira.md")).unwrap();
    body.push_str(LEDGER);
    body.push('\n');
    fs::write(owner.join("cast/mira.md"), body).unwrap();
    let hits = spookery_kernel::lint::lint(&owner).unwrap();
    assert!(hits.iter().any(|hit| hit.path == "cast/mira.md" && hit.text == LEDGER));
    let place = temp_pack("lint-place");
    let mut body = fs::read_to_string(place.join("world/places/docks_shop.md")).unwrap();
    body.push_str(LEDGER);
    body.push('\n');
    fs::write(place.join("world/places/docks_shop.md"), body).unwrap();
    let hits = spookery_kernel::lint::lint(&place).unwrap();
    assert!(hits
        .iter()
        .any(|hit| hit.path == "world/places/docks_shop.md" && hit.text == LEDGER));
    let folded = temp_pack("lint-case");
    let mut body = fs::read_to_string(folded.join("cast/mira.md")).unwrap();
    body.push_str("the lamp-oil ledger is short.\n");
    fs::write(folded.join("cast/mira.md"), body).unwrap();
    assert!(spookery_kernel::lint::lint(&folded).unwrap().is_empty());
    let _ = fs::remove_dir_all(&pack);
    let _ = fs::remove_dir_all(&owner);
    let _ = fs::remove_dir_all(&place);
    let _ = fs::remove_dir_all(&folded);
}

#[test]
fn import_boundary() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut kernel_tick = Vec::new();
    fn files_under(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                files_under(&path, out);
            } else {
                out.push(path);
            }
        }
    }
    files_under(&root.join("src/kernel"), &mut kernel_tick);
    files_under(&root.join("src/tick"), &mut kernel_tick);
    let hits = |paths: &[PathBuf], needles: &[&str]| -> Vec<String> {
        let mut found = Vec::new();
        for path in paths {
            let bytes = fs::read(path).unwrap();
            for needle in needles {
                let needle_bytes = needle.as_bytes();
                if bytes.windows(needle_bytes.len()).any(|window| window == needle_bytes) {
                    found.push(format!("{} contains {needle}", path.display()));
                }
            }
        }
        found
    };
    let kernel_hits = hits(&kernel_tick, &["rmcp", "tokio", "sqlx", "postgres", "redis"]);
    assert!(kernel_hits.is_empty(), "{}", kernel_hits.join("\n"));
    let mut src_files = Vec::new();
    files_under(&root.join("src"), &mut src_files);
    let src_hits = hits(
        &src_files,
        &["langchain", "openai", "anthropic", "ollama", "reqwest"],
    );
    assert!(src_hits.is_empty(), "{}", src_hits.join("\n"));
}

fn player_waits(tick: &Value) -> Vec<String> {
    tick["intents"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|intent| intent["move"] == "wait" && intent["source"] == "player")
        .map(|intent| intent["id"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn queued_line_survives_a_dropped_seat() {
    let held = temp_pack("held");
    let mut shard = open(&held);
    let session = seat_player(&mut shard, "pc_rowan").unwrap();
    assert!(!on_scheduled_wake(&mut shard, true, false).unwrap());
    let note = queue_wait(&mut shard, "pc_rowan").unwrap();
    assert!(!on_scheduled_wake(&mut shard, true, true).unwrap());
    assert_eq!(shard.queued_intents().len(), 1);
    assert_eq!(shard.queued_intents()[0].id, note.id);
    assert_eq!(shard.now().world_time, 1080);
    assert!(tick_names(&held).is_empty());
    assert!(release_player_seat(&mut shard, &session, "pc_rowan", SocketEnd::Close).unwrap());
    assert_eq!(shard.now().presence.humans, 0);
    assert!(!on_scheduled_wake(&mut shard, false, true).unwrap());
    assert_eq!(shard.queued_intents().len(), 1);
    assert_eq!(shard.now().tick_index, 0);
    assert!(tick_names(&held).is_empty());
    let _ = fs::remove_dir_all(&held);

    let dropped = temp_pack("drop-queue");
    let mut shard = open(&dropped);
    let session = seat_player(&mut shard, "pc_rowan").unwrap();
    let note = turn(&mut shard, "pc_rowan", "wait", true).unwrap();
    let Turn::Queued(note) = note else {
        panic!("queued wait committed");
    };
    assert!(release_player_seat(&mut shard, &session, "pc_rowan", SocketEnd::Close).unwrap());
    assert_eq!(shard.now().world_time, 1080);
    assert_eq!(shard.now().tick_index, 0);
    assert_eq!(shard.queued_intents().len(), 1);
    assert!(tick_names(&dropped).is_empty());
    seat_player(&mut shard, "pc_rowan").unwrap();
    assert_eq!(shard.now().world_time, 1080);
    assert_eq!(shard.now().tick_index, 0);
    assert_eq!(shard.queued_intents()[0].id, note.id);
    assert!(tick_names(&dropped).is_empty());
    let _ = fs::remove_dir_all(&dropped);
}

#[test]
fn early_wake_does_not_tick_again() {
    let early = temp_pack("early-wake");
    let mut shard = open(&early);
    let session = seat_player(&mut shard, "pc_rowan").unwrap();
    queue_wait(&mut shard, "pc_rowan").unwrap();
    assert!(commit_queued(&mut shard).unwrap());
    let committed = shard.now().clone();
    assert_eq!(committed.world_time, 1085);
    assert!(release_player_seat(&mut shard, &session, "pc_rowan", SocketEnd::Close).unwrap());
    assert!(!on_scheduled_wake(&mut shard, false, false).unwrap());
    assert_eq!(shard.now().world_time, committed.world_time);
    assert_eq!(shard.now().tick_index, committed.tick_index);
    assert_eq!(tick_names(&early), vec!["tick-0001.jsonl".to_string()]);
    let _ = fs::remove_dir_all(&early);
}

#[test]
fn a_due_wake_applies_the_queued_wait() {
    let due = temp_pack("due-wake");
    let mut shard = open(&due);
    let note = queue_wait(&mut shard, "pc_rowan").unwrap();
    assert_eq!(shard.now().last_tick_at, "2026-01-01T00:00:00Z");
    assert!(on_scheduled_wake(&mut shard, false, false).unwrap());
    assert_eq!(shard.now().tick_index, 12);
    assert_eq!(shard.now().world_time, 1800);
    assert!(shard.queued_intents().is_empty());
    assert_eq!(
        player_waits(&tick_value(&due, "tick-0001.jsonl")),
        vec![note.id]
    );
    assert!(player_waits(&tick_value(&due, "tick-0002.jsonl")).is_empty());
    assert!(!on_scheduled_wake(&mut shard, false, false).unwrap());
    assert_eq!(shard.now().tick_index, 12);
    let _ = fs::remove_dir_all(&due);
}

fn patch_mira_flags(pack: &Path, flags: &str) {
    let path = pack.join("cast/mira.md");
    let text = fs::read_to_string(&path).unwrap();
    let next = text.replacen(
        "tick:\n  eligible: true\n",
        &format!("tick:\n  eligible: true\n  flags:\n{flags}"),
        1,
    );
    assert_ne!(next, text, "mira tick block was not patched");
    fs::write(&path, next).unwrap();
}

fn set_grit(pack: &Path, grit: i64) {
    if grit == 1 {
        return;
    }
    let path = pack.join("state/actors/pc_rowan.yaml");
    let text = fs::read_to_string(&path).unwrap();
    let next = text.replacen("stats:\n  grit: 1\n", &format!("stats:\n  grit: {grit}\n"), 1);
    assert_ne!(next, text, "grit was not patched");
    fs::write(&path, next).unwrap();
}

fn play_mark(pack: &Path, line: &str) -> Now {
    let mut shard = open(pack);
    seat_player(&mut shard, "pc_rowan").unwrap();
    let played = turn(&mut shard, "pc_rowan", line, false).unwrap();
    let Turn::Committed { .. } = played else {
        panic!("{line} stayed queued");
    };
    shard.now().clone()
}

#[test]
fn effects_run_on_both_outcomes_and_lists_run_on_one() {
    for (label, grit, success) in [("effects-pass", 1, true), ("effects-fail", -20, false)] {
        let pack = temp_pack(label);
        let mut moves = fs::read_to_string(pack.join("rules/moves.yaml")).unwrap();
        moves.push_str(
            "\n  - name: mark\n    audience: player\n    token: actor\n    on: named\n    check: grit\n    effects:\n      - class: flag\n        key: marked\n        value: true\n",
        );
        fs::write(pack.join("rules/moves.yaml"), moves).unwrap();
        patch_mira_flags(&pack, "    - marked\n");
        set_grit(&pack, grit);
        stamp_pack_rev(&pack);
        let now = play_mark(&pack, "mark mira");
        assert_eq!(now.world_time, 1085);
        assert_eq!(flags_of(&now, "mira"), vec!["marked".to_string()], "{label} success {success}");
        let tick = tick_value(&pack, "tick-0001.jsonl");
        assert!(tick["tool_trace"].as_array().unwrap().iter().any(|name| name == "resolve_check"));
        assert!(!fs::read_to_string(pack.join("state/log/tick-0001.jsonl")).unwrap().contains("faces"));
        let _ = fs::remove_dir_all(&pack);
    }

    for (label, grit, flag) in [("lists-pass", 1, "marked"), ("lists-fail", -20, "missed")] {
        let pack = temp_pack(label);
        let mut moves = fs::read_to_string(pack.join("rules/moves.yaml")).unwrap();
        moves.push_str(
            "\n  - name: mark\n    audience: player\n    token: actor\n    on: named\n    check: grit\n    success:\n      - class: flag\n        key: marked\n        value: true\n    failure:\n      - class: flag\n        key: missed\n        value: true\n",
        );
        fs::write(pack.join("rules/moves.yaml"), moves).unwrap();
        patch_mira_flags(&pack, "    - marked\n    - missed\n");
        set_grit(&pack, grit);
        stamp_pack_rev(&pack);
        let now = play_mark(&pack, "mark mira");
        assert_eq!(flags_of(&now, "mira"), vec![flag.to_string()], "{label}");
        assert_eq!(now.world_time, 1085);
        let _ = fs::remove_dir_all(&pack);
    }
}

fn insert_nell(pack: &Path) {
    write_text(
        &pack.join("cast/pc-nell.md"),
        "\
---
schema: cast/v0
id: pc_nell
kind: pc_template
facts: []
tick:
  eligible: false
  flags:
    - marked
    - missed
    - guarded
---
Nell keeps a guard up.
",
    );
    write_text(
        &pack.join("state/actors/pc_nell.yaml"),
        "\
schema: sheet/v0
id: pc_nell
template: cast/pc-nell.md
stats:
  grit: 100
resources: {}
inventory: []
flags: []
",
    );
    let now_path = pack.join("state/now.yaml");
    let mut now = spookery_kernel::codec::load_now(&fs::read_to_string(&now_path).unwrap()).unwrap();
    now.actors.insert(
        "pc_nell".to_string(),
        ActorLive::Pc {
            place: "docks_shop".to_string(),
            flags: Vec::new(),
            busy: false,
            known_facts: Vec::new(),
            sheet: "state/actors/pc_nell.yaml".to_string(),
        },
    );
    fs::write(&now_path, emit_now(&now).unwrap()).unwrap();
}

fn rolled_stat(notices: &[CheckNotice], stat: i64) -> spookery_kernel::check::CheckOut {
    notices
        .iter()
        .find_map(|notice| match notice {
            CheckNotice::Rolled { out, .. } if out.stat == stat => Some(out.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no stat {stat} in {notices:?}"))
}

#[test]
fn opposing_total_reused() {
    for opposed in [false, true] {
        let label = if opposed { "opposed" } else { "open-check" };
        let pack = temp_pack(label);
        let mut moves = fs::read_to_string(pack.join("rules/moves.yaml")).unwrap();
        moves.push_str(
            "\n  - name: mark\n    audience: player\n    token: actor\n    on: named\n    check: grit\n    success:\n      - class: flag\n        key: marked\n        value: true\n    failure:\n      - class: flag\n        key: missed\n        value: true\n  - name: defend\n    audience: player\n    token: none\n    on: speaker\n    check: grit\n    success:\n      - class: flag\n        key: guarded\n        value: true\n",
        );
        fs::write(pack.join("rules/moves.yaml"), moves).unwrap();
        insert_nell(&pack);
        stamp_pack_rev(&pack);
        let mut shard = open(&pack);
        seat_player(&mut shard, "pc_rowan").unwrap();
        if opposed {
            let args = json!({
                "actor_id": "pc_nell",
                "actor_kind": "player",
                "source": "player",
                "move": "defend"
            });
            let queued = gm_tool(&mut shard, "intent", Some(args.as_object().unwrap().clone()));
            assert!(queued.value.get("code").is_none(), "{}", queued.value);
        }
        let mark = turn(&mut shard, "pc_rowan", "mark pc_nell", true).unwrap();
        let Turn::Queued(_) = mark else {
            panic!("mark committed while a gm seat was requested");
        };
        assert_eq!(shard.now().world_time, 1080);
        assert!(commit_queued(&mut shard).unwrap());
        let notices = shard.take_rolls();
        let rowan = rolled_stat(&notices, 1);
        assert_eq!(shard.now().world_time, 1085);
        let tick = fs::read_to_string(pack.join("state/log/tick-0001.jsonl")).unwrap();
        assert!(!tick.contains("faces"));
        if opposed {
            let nell = rolled_stat(&notices, 100);
            assert_eq!(nell.target, 0);
            assert!(nell.success);
            assert!(nell.total >= 102, "{nell:?}");
            assert_eq!(rowan.target, nell.total);
            assert!(!rowan.success);
            let mut flags = flags_of(shard.now(), "pc_nell");
            flags.sort();
            assert_eq!(flags, vec!["guarded".to_string(), "missed".to_string()]);
        } else {
            assert_eq!(rowan.target, 0);
            assert!(rowan.success);
            assert_eq!(flags_of(shard.now(), "pc_nell"), vec!["marked".to_string()]);
        }
        let _ = fs::remove_dir_all(&pack);
    }
}

#[test]
fn a_held_knife_still_runs_the_opposed_row() {
    let held = temp_pack("item-held");
    let mut moves = fs::read_to_string(held.join("rules/moves.yaml")).unwrap();
    moves.push_str(
        "\n  - name: mark\n    audience: player\n    token: actor\n    on: named\n    check: grit\n    item: knife\n    success:\n      - class: flag\n        key: marked\n        value: true\n    failure:\n      - class: flag\n        key: missed\n        value: true\n  - name: defend\n    audience: player\n    token: none\n    on: speaker\n    check: grit\n    success:\n      - class: flag\n        key: guarded\n        value: true\n",
    );
    fs::write(held.join("rules/moves.yaml"), moves).unwrap();
    insert_nell(&held);
    let sheet = held.join("state/actors/pc_rowan.yaml");
    let text = fs::read_to_string(&sheet).unwrap();
    let next = text.replacen("inventory: []\n", "inventory:\n  - knife\n", 1);
    assert_ne!(next, text);
    fs::write(&sheet, next).unwrap();
    stamp_pack_rev(&held);
    let mut shard = open(&held);
    seat_player(&mut shard, "pc_rowan").unwrap();
    let args = json!({
        "actor_id": "pc_nell",
        "actor_kind": "player",
        "source": "player",
        "move": "defend"
    });
    gm_tool(&mut shard, "intent", Some(args.as_object().unwrap().clone()));
    let queued = turn(&mut shard, "pc_rowan", "mark pc_nell", true).unwrap();
    let Turn::Queued(_) = queued else {
        panic!("held mark committed before the round");
    };
    assert!(commit_queued(&mut shard).unwrap());
    let rowan = rolled_stat(&shard.take_rolls(), 1);
    assert!(!rowan.success);
    assert!(!fs::read_to_string(held.join("state/log/tick-0001.jsonl")).unwrap().contains("faces"));
    let _ = fs::remove_dir_all(&held);
}

#[test]
fn manifests_do_not_name_a_model_crate() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for path in [
        root.join("Cargo.toml"),
        root.join("src/kernel/Cargo.toml"),
        root.join("src/mcp/Cargo.toml"),
        root.join("src/tick/Cargo.toml"),
    ] {
        let text = fs::read_to_string(&path).unwrap();
        assert!(!text.contains("llm"), "{}", path.display());
    }
}
