use cucumber::when;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use spookery::seat::{
    commit_queued, gm_tool, on_scheduled_wake, queue_wait, release_player_seat, seat_player, turn,
    SocketEnd, Turn,
};
use spookery_kernel::apply::apply_effect;
use spookery_kernel::check::CheckNotice;
use spookery_kernel::codec::emit_now;
use spookery_kernel::effect::{Effect, EffectClass};
use spookery_kernel::intent::{ActorKind, Intent, IntentArgs, Source, Target, ViewerId};
use spookery_kernel::meter::MeterLive;
use spookery_kernel::model::{
    ActorLive, Message, Now, PauseView, PlaceLive, Presence, RunState, TickCounters,
};
use spookery_kernel::shard::{Shard, Txn};
use spookery_tick::{replay_recorded, run_catchup, run_tick, tick_pack};

use crate::World;

const LEDGER: &str = "The lamp-oil ledger is short.";

fn meters(grit: i64) -> BTreeMap<String, MeterLive> {
    BTreeMap::from([
        (
            "grit".to_string(),
            MeterLive {
                base: grit,
                offset: 0,
            },
        ),
        ("rest".to_string(), MeterLive { base: 3, offset: 0 }),
    ])
}

fn copy_dir(from: &Path, to: &Path) {
    super::support::copy_dir(from, to);
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
    let mut now =
        spookery_kernel::codec::load_now(&fs::read_to_string(&now_path).unwrap()).unwrap();
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
}

fn door_message() -> Message {
    Message {
        id: "note_mira_door".to_string(),
        to: vec!["pc_rowan".to_string()],
        place: Some("docks_shop".to_string()),
        fact_ids: Vec::new(),
        text: "Back at dusk. Oil on the shelf.".to_string(),
    }
}

fn save_now(pack: &Path, now: &Now) {
    fs::write(pack.join("state/now.yaml"), emit_now(now).unwrap()).unwrap();
}

fn evening_note() {
    let pack = temp_pack("evening");
    tick_pack(&pack).unwrap_or_else(|err| panic!("tick: {err}"));
    let now = now_of(&pack);
    assert_eq!(lamp_oil(&now), 4);
    assert_eq!(now.actors.get("mira").unwrap().place(), "mira_rooms");
    assert_eq!(now.messages.len(), 1);
    assert_eq!(now.messages[0].id, "note_mira_door");
    assert_eq!(now.messages[0].place.as_deref(), Some("docks_shop"));
    let _ = fs::remove_dir_all(&pack);
}

fn morning_restock() {
    let pack = temp_pack("morning");
    {
        let shard = open(&pack);
        let mut now = shard.now().clone();
        drop(shard);
        now.world_time = 600;
        now.messages.push(door_message());
        *now.actors.get_mut("pc_rowan").unwrap().place_mut() = "docks_street".to_string();
        save_now(&pack, &now);
    }
    assert_eq!(
        now_of(&pack).actors.get("mira").unwrap().place(),
        "mira_rooms"
    );
    tick_pack(&pack).unwrap_or_else(|err| panic!("tick: {err}"));
    let now = now_of(&pack);
    assert_eq!(lamp_oil(&now), 8);
    assert_eq!(now.actors.get("mira").unwrap().place(), "mira_rooms");
    assert_eq!(now.messages.len(), 1);
    let _ = fs::remove_dir_all(&pack);
}

fn off_hours_hold() {
    let pack = temp_pack("off-hours");
    {
        let shard = open(&pack);
        let mut now = shard.now().clone();
        drop(shard);
        now.messages.push(door_message());
        save_now(&pack, &now);
    }
    tick_pack(&pack).unwrap_or_else(|err| panic!("tick: {err}"));
    let now = now_of(&pack);
    assert_eq!(lamp_oil(&now), 4);
    assert_eq!(now.messages.len(), 1);
    let _ = fs::remove_dir_all(&pack);
}

fn player_in_the_shop() {
    let pack = temp_pack("present");
    {
        let shard = open(&pack);
        let mut now = shard.now().clone();
        drop(shard);
        now.world_time = 600;
        now.presence.humans = 1;
        now.presence.actors_present = vec!["pc_rowan".to_string()];
        now.messages.push(door_message());
        *now.actors.get_mut("mira").unwrap().place_mut() = "docks_shop".to_string();
        save_now(&pack, &now);
    }
    tick_pack(&pack).unwrap_or_else(|err| panic!("tick: {err}"));
    let now = now_of(&pack);
    assert_eq!(lamp_oil(&now), 4);
    assert_eq!(now.messages.len(), 1);
    assert_eq!(
        now.actors
            .get("mira")
            .unwrap()
            .tick()
            .unwrap()
            .last_act_tick,
        0
    );
    let _ = fs::remove_dir_all(&pack);
}

fn gm_restocks() {
    let pack = temp_pack("gm-restock");
    {
        let shard = open(&pack);
        let mut now = shard.now().clone();
        drop(shard);
        now.world_time = 600;
        now.presence.humans = 1;
        now.presence.actors_present = vec!["pc_rowan".to_string()];
        now.messages.push(door_message());
        *now.actors.get_mut("mira").unwrap().place_mut() = "docks_shop".to_string();
        save_now(&pack, &now);
    }
    let mut shard = open(&pack);
    let wall = shard.now().last_tick_at.clone();
    let args = json!({
        "actor_id": "mira",
        "actor_kind": "npc",
        "source": "player",
        "move": "restock_lamp_oil"
    });
    let queued = gm_tool(
        &mut shard,
        "intent",
        Some(args.as_object().unwrap().clone()),
    );
    assert!(queued.value.get("code").is_none(), "{}", queued.value);
    run_tick(&mut shard, spookery_tick::Cause::Manual, &wall).unwrap();
    assert_eq!(lamp_oil(shard.now()), 8);
    assert_eq!(shard.now().messages.len(), 1);
    let _ = fs::remove_dir_all(&pack);
}

fn claimed_note_sorts_first() {
    let pack = temp_pack("claimed");
    {
        let shard = open(&pack);
        let mut now = shard.now().clone();
        drop(shard);
        now.presence.humans = 1;
        now.presence.actors_present = vec!["pc_rowan".to_string()];
        *now.actors.get_mut("mira").unwrap().place_mut() = "docks_shop".to_string();
        save_now(&pack, &now);
    }
    let mut shard = open(&pack);
    let wall = shard.now().last_tick_at.clone();
    for (actor, move_name) in [("bram", "wait"), ("mira", "leave_door_note")] {
        let args = json!({
            "actor_id": actor,
            "actor_kind": "npc",
            "source": "player",
            "move": move_name
        });
        let queued = gm_tool(
            &mut shard,
            "intent",
            Some(args.as_object().unwrap().clone()),
        );
        assert!(queued.value.get("code").is_none(), "{}", queued.value);
    }
    run_tick(&mut shard, spookery_tick::Cause::Manual, &wall).unwrap();
    let log = tick_value(&pack, "tick-0001.jsonl");
    let moves: Vec<&str> = log["intents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|intent| intent["move"].as_str().unwrap())
        .collect();
    assert_eq!(moves, vec!["leave_door_note", "wait"]);
    assert_eq!(log["intents"][0]["args"]["to"][0].as_str(), Some("pc_rowan"));
    let _ = fs::remove_dir_all(&pack);
}

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
    for name in ["moves.yaml", "checks.yaml", "meters.yaml", "tick.yaml"] {
        fs::copy(rules.join(name), {
            let dest = pack.join("rules").join(name);
            fs::create_dir_all(dest.parent().unwrap()).unwrap();
            dest
        })
        .unwrap_or_else(|err| panic!("copy {name}: {err}"));
    }
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
            meters: meters(1),
            inventory: Vec::new(),
            known_facts: Vec::new(),
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
        },
        places,
        actors,
        clocks: BTreeMap::new(),
        rumors: Vec::new(),
        messages: Vec::new(),
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
    assert_eq!(tick_names(&pack).len(), 12);
    assert_eq!(
        fs::read(pack.join("world/places/gate.md")).unwrap(),
        place_before
    );
    assert_eq!(
        fs::read(pack.join("cast/pc_rowan.md")).unwrap(),
        cast_before
    );
    let _ = fs::remove_dir_all(&pack);
}

fn busy_skip() {
    let pack = temp_pack("busy");
    {
        let shard = open(&pack);
        let mut now = shard.now().clone();
        drop(shard);
        now.world_time = 600;
        now.messages.push(door_message());
        *now.actors.get_mut("mira").unwrap().busy_mut() = true;
        *now.actors.get_mut("pc_rowan").unwrap().place_mut() = "docks_street".to_string();
        save_now(&pack, &now);
    }
    tick_pack(&pack).unwrap_or_else(|err| panic!("tick: {err}"));
    let now = now_of(&pack);
    assert_eq!(lamp_oil(&now), 4);
    assert_eq!(
        now.actors
            .get("mira")
            .unwrap()
            .tick()
            .unwrap()
            .last_act_tick,
        0
    );
    assert_eq!(
        now.actors
            .get("bram")
            .unwrap()
            .tick()
            .unwrap()
            .last_act_tick,
        0
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

fn an_open_transaction_writes_nothing() {
    let dropped = temp_pack("drop");
    let now_path = dropped.join("state/now.yaml");
    let before = fs::read(&now_path).unwrap();
    let shard = open(&dropped);
    let state_rev = shard.now().state_rev;
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
    assert!(!dropped.join("state/memories").exists());
    let _ = fs::remove_dir_all(&dropped);
}

fn an_incomplete_transaction_is_discarded() {
    let incomplete = temp_pack("incomplete");
    let now_path = incomplete.join("state/now.yaml");
    let before = fs::read(&now_path).unwrap();
    let txn = incomplete.join("state/.txn");
    fs::create_dir_all(txn.join("prev")).unwrap();
    fs::write(txn.join("prev/now.yaml"), b"not-yaml\n").unwrap();
    let opened = open(&incomplete);
    assert_eq!(fs::read(&now_path).unwrap(), before);
    assert_eq!(
        lamp_oil(opened.now()),
        lamp_oil(&spookery_kernel::codec::load_now(std::str::from_utf8(&before).unwrap()).unwrap())
    );
    assert!(!txn.exists());
    let _ = fs::remove_dir_all(&incomplete);
}

fn the_same_revision_restores_the_previous_save() {
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
        lamp_oil(
            &spookery_kernel::codec::load_now(std::str::from_utf8(&original).unwrap()).unwrap()
        )
    );
    assert!(!txn.exists());
    let _ = fs::remove_dir_all(&same);
}

fn the_next_revision_keeps_the_new_save() {
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
        format!(
            "{{\"tick_index\":1,\"state_rev_after\":{}}}\n",
            live.state_rev
        ),
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
    let opened = open(&next);
    assert_eq!(opened.now().state_rev, live.state_rev);
    assert_eq!(lamp_oil(opened.now()), kept_oil);
    assert!(!txn.exists());
    assert!(log.join("tick-0001.jsonl").is_file());
    assert!(!log.join("tick-0099.jsonl").exists());
    let memory = fs::read_to_string(memories.join("mira.jsonl")).unwrap();
    assert!(memory.contains("kept-line"));
    assert!(!memory.contains("orphan-line"));
    let _ = fs::remove_dir_all(&next);
}

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
    assert_eq!(
        fs::read(played.join("state/log/tick-0020.jsonl")).unwrap(),
        source_log
    );
    let _ = fs::remove_dir_all(&start);
    let _ = fs::remove_dir_all(&played);
    let _ = fs::remove_dir_all(&temp);
}

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
  range:
    - yard
  habits:
    - id: hold_still
      runs_while_empty: true
      rows:
        - hold
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
                meters: meters(1),
                inventory: Vec::new(),
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
    for name in ["moves.yaml", "checks.yaml", "meters.yaml", "tick.yaml"] {
        let dest = pack.join("rules").join(name);
        fs::create_dir_all(dest.parent().unwrap()).unwrap();
        fs::copy(rules.join(name), &dest).unwrap();
    }
    let moves_path = pack.join("rules/moves.yaml");
    let mut moves = fs::read_to_string(&moves_path).unwrap();
    moves.push_str(
        "\n  - name: hold\n    audience: habit\n    token: none\n    on: speaker\n    effects: []\n",
    );
    fs::write(&moves_path, moves).unwrap();
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
        },
        places,
        actors,
        clocks: BTreeMap::new(),
        rumors: Vec::new(),
        messages: Vec::new(),
    };
    write_text(&pack.join("state/now.yaml"), &emit_now(&now).unwrap());
    tick_pack(&pack).unwrap_or_else(|err| panic!("tick: {err}"));
    let now = now_of(&pack);
    assert_eq!(now.tick_index, 1);
    assert_eq!(
        now.actors.values().filter(|actor| actor.is_npc()).count(),
        50
    );
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
    assert!(!capped
        .unknown_mentioned
        .iter()
        .any(|id| id == "fact_ledger_short"));
    assert!(capped
        .unknown_mentioned
        .iter()
        .any(|id| id == "fact_room_lamp"));
    drop(shard);
    assert_eq!(snapshot(&pack), before);
    let _ = fs::remove_dir_all(&pack);
}

fn markdown_files(dir: &Path, out: &mut Vec<PathBuf>) {
    if !dir.exists() {
        return;
    }
    let mut entries: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap())
        .collect();
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

fn strip_bodies_and_hidden_string_lint() {
    let pack = temp_pack("strip");
    let mut files = Vec::new();
    markdown_files(&pack.join("cast"), &mut files);
    markdown_files(&pack.join("world"), &mut files);
    assert!(!files.is_empty());
    for path in &files {
        let text = fs::read_to_string(path).unwrap();
        let stripped = spookery_kernel::lint::strip_body(&text).unwrap();
        assert!(
            !stripped.contains("Mira keeps the shop"),
            "{}",
            path.display()
        );
        assert!(!stripped.contains("Bram walks"), "{}", path.display());
        fs::write(path, stripped).unwrap();
    }
    let mira = fs::read_to_string(pack.join("cast/mira.md")).unwrap();
    assert!(mira.contains(LEDGER), "frontmatter secret removed");
    assert!(!fs::read_to_string(pack.join("cast/aldric.md"))
        .unwrap()
        .contains(LEDGER));
    stamp_pack_rev(&pack);
    open(&pack);
    tick_pack(&pack).unwrap_or_else(|err| panic!("stripped pack did not tick: {err}"));
    let perceive = format!(
        "{:?}",
        spookery_kernel::perceive(&open(&pack), "npc:aldric").unwrap()
    );
    assert!(!perceive.contains(LEDGER));

    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("packs/oneshot-docks");
    assert!(spookery_kernel::lint::lint(&fixture).unwrap().is_empty());
    let owner = temp_pack("lint-owner");
    let mut body = fs::read_to_string(owner.join("cast/mira.md")).unwrap();
    body.push_str(LEDGER);
    body.push('\n');
    fs::write(owner.join("cast/mira.md"), body).unwrap();
    let hits = spookery_kernel::lint::lint(&owner).unwrap();
    assert!(hits
        .iter()
        .any(|hit| hit.path == "cast/mira.md" && hit.text == LEDGER));
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
                if bytes
                    .windows(needle_bytes.len())
                    .any(|window| window == needle_bytes)
                {
                    found.push(format!("{} contains {needle}", path.display()));
                }
            }
        }
        found
    };
    let kernel_hits = hits(
        &kernel_tick,
        &["rmcp", "tokio", "sqlx", "postgres", "redis"],
    );
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
}

fn a_reseated_player_keeps_the_queued_wait() {
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

fn play_mark(pack: &Path, line: &str) -> Now {
    let mut shard = open(pack);
    seat_player(&mut shard, "pc_rowan").unwrap();
    let played = turn(&mut shard, "pc_rowan", line, false).unwrap();
    let Turn::Committed { .. } = played else {
        panic!("{line} stayed queued");
    };
    shard.now().clone()
}

fn effects_run_on_both_outcomes_and_lists_run_on_one() {
    for (label, difficulty, success) in [("effects-pass", 0, true), ("effects-fail", 14, false)] {
        let pack = temp_pack(label);
        let mut moves = fs::read_to_string(pack.join("rules/moves.yaml")).unwrap();
        moves.push_str(&format!(
            "\n  - name: mark\n    audience: player\n    token: actor\n    on: named\n    check: grit\n    difficulty: {difficulty}\n    effects:\n      - class: flag\n        key: marked\n        value: true\n",
        ));
        fs::write(pack.join("rules/moves.yaml"), moves).unwrap();
        patch_mira_flags(&pack, "    - marked\n");
        stamp_pack_rev(&pack);
        let now = play_mark(&pack, "mark mira");
        assert_eq!(now.world_time, 1085);
        assert_eq!(
            flags_of(&now, "mira"),
            vec!["marked".to_string()],
            "{label} success {success}"
        );
        let tick = tick_value(&pack, "tick-0001.jsonl");
        assert!(tick["tool_trace"]
            .as_array()
            .unwrap()
            .iter()
            .any(|name| name == "resolve_check"));
        assert!(!fs::read_to_string(pack.join("state/log/tick-0001.jsonl"))
            .unwrap()
            .contains("faces"));
        let _ = fs::remove_dir_all(&pack);
    }

    for (label, difficulty, flag) in [("lists-pass", 0, "marked"), ("lists-fail", 14, "missed")] {
        let pack = temp_pack(label);
        let mut moves = fs::read_to_string(pack.join("rules/moves.yaml")).unwrap();
        moves.push_str(&format!(
            "\n  - name: mark\n    audience: player\n    token: actor\n    on: named\n    check: grit\n    difficulty: {difficulty}\n    success:\n      - class: flag\n        key: marked\n        value: true\n    failure:\n      - class: flag\n        key: missed\n        value: true\n",
        ));
        fs::write(pack.join("rules/moves.yaml"), moves).unwrap();
        patch_mira_flags(&pack, "    - marked\n    - missed\n");
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
    let now_path = pack.join("state/now.yaml");
    let mut now =
        spookery_kernel::codec::load_now(&fs::read_to_string(&now_path).unwrap()).unwrap();
    now.actors.insert(
        "pc_nell".to_string(),
        ActorLive::Pc {
            place: "docks_shop".to_string(),
            flags: Vec::new(),
            busy: false,
            meters: meters(100),
            inventory: Vec::new(),
            known_facts: Vec::new(),
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
            let queued = gm_tool(
                &mut shard,
                "intent",
                Some(args.as_object().unwrap().clone()),
            );
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

fn a_held_knife_still_runs_the_opposed_row() {
    let held = temp_pack("item-held");
    let mut moves = fs::read_to_string(held.join("rules/moves.yaml")).unwrap();
    moves.push_str(
        "\n  - name: mark\n    audience: player\n    token: actor\n    on: named\n    check: grit\n    item: knife\n    success:\n      - class: flag\n        key: marked\n        value: true\n    failure:\n      - class: flag\n        key: missed\n        value: true\n  - name: defend\n    audience: player\n    token: none\n    on: speaker\n    check: grit\n    success:\n      - class: flag\n        key: guarded\n        value: true\n",
    );
    fs::write(held.join("rules/moves.yaml"), moves).unwrap();
    insert_nell(&held);
    let now_path = held.join("state/now.yaml");
    let text = fs::read_to_string(&now_path).unwrap();
    let next = text.replacen(
        "  pc_rowan:\n    kind: pc\n    place: docks_shop\n    flags: []\n    busy: false\n    meters:\n      grit:\n        base: 1\n        offset: 0\n      rest:\n        base: 3\n        offset: 0\n    inventory: []\n",
        "  pc_rowan:\n    kind: pc\n    place: docks_shop\n    flags: []\n    busy: false\n    meters:\n      grit:\n        base: 1\n        offset: 0\n      rest:\n        base: 3\n        offset: 0\n    inventory:\n      - knife\n",
        1,
    );
    assert_ne!(next, text);
    fs::write(&now_path, next).unwrap();
    stamp_pack_rev(&held);
    let mut shard = open(&held);
    seat_player(&mut shard, "pc_rowan").unwrap();
    let args = json!({
        "actor_id": "pc_nell",
        "actor_kind": "player",
        "source": "player",
        "move": "defend"
    });
    gm_tool(
        &mut shard,
        "intent",
        Some(args.as_object().unwrap().clone()),
    );
    let queued = turn(&mut shard, "pc_rowan", "mark pc_nell", true).unwrap();
    let Turn::Queued(_) = queued else {
        panic!("held mark committed before the round");
    };
    assert!(commit_queued(&mut shard).unwrap());
    let rowan = rolled_stat(&shard.take_rolls(), 1);
    assert!(!rowan.success);
    assert!(!fs::read_to_string(held.join("state/log/tick-0001.jsonl"))
        .unwrap()
        .contains("faces"));
    let _ = fs::remove_dir_all(&held);
}

fn the_scheduler_does_not_spawn_a_tick() {
    let source =
        fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/schedule.rs"))
            .unwrap();
    assert!(
        source.contains("tokio::time::sleep"),
        "scheduler does not sleep on a Tokio timer"
    );
    assert!(
        !source.contains("tokio::spawn"),
        "scheduler spawns a tick instead of calling run_tick on the calling thread"
    );
}

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

#[when(expr = "twelve empty ticks pass")]
fn twelve_empty_ticks_pass(_world: &mut World) {
    no_self_win();
}

#[when(expr = "the evening tick runs with Rowan away")]
fn the_evening_tick_runs(_world: &mut World) {
    evening_note();
}

#[when(expr = "the morning tick runs with the note already left")]
fn the_morning_tick_runs(_world: &mut World) {
    morning_restock();
}

#[when(expr = "an off-hours tick runs with the note already left")]
fn an_off_hours_tick_runs(_world: &mut World) {
    off_hours_hold();
}

#[when(expr = "Rowan stands with Mira during shop hours")]
fn rowan_stands_with_mira(_world: &mut World) {
    player_in_the_shop();
}

#[when(expr = "the GM queues Mira's restock in front of Rowan")]
fn the_gm_queues_miras_restock(_world: &mut World) {
    gm_restocks();
}

#[when(expr = "the GM queues the door note ahead of a wait")]
fn the_gm_queues_the_door_note(_world: &mut World) {
    claimed_note_sorts_first();
}

#[when(expr = "seventy-two hours of catch-up run")]
fn seventy_two_hours(_world: &mut World) {
    catchup_cap();
}

#[when(expr = "Mira is busy and a tick runs")]
fn mira_is_busy(_world: &mut World) {
    busy_skip();
}

#[when(expr = "an open transaction is dropped")]
fn an_open_transaction_is_dropped(_world: &mut World) {
    an_open_transaction_writes_nothing();
}

#[when(expr = "an incomplete transaction is opened")]
fn an_incomplete_transaction_is_opened(_world: &mut World) {
    an_incomplete_transaction_is_discarded();
}

#[when(expr = "a finished transaction repeats the current revision")]
fn a_finished_transaction_repeats(_world: &mut World) {
    the_same_revision_restores_the_previous_save();
}

#[when(expr = "a finished transaction advances the revision")]
fn a_finished_transaction_advances(_world: &mut World) {
    the_next_revision_keeps_the_new_save();
}

#[when(expr = "twenty ticks are replayed")]
fn twenty_ticks_are_replayed(_world: &mut World) {
    replay_byte_compares_now();
}

#[when(expr = "fifty NPCs share one tick")]
fn fifty_npcs(_world: &mut World) {
    budget_caps();
}

#[when(expr = "an npc perceive is called with a zero budget")]
fn an_npc_perceive_is_called(_world: &mut World) {
    tool_scope();
}

#[when(expr = "prose is stripped and a hidden sentence is repeated")]
fn prose_is_stripped(_world: &mut World) {
    strip_bodies_and_hidden_string_lint();
}

#[when(expr = "the kernel and tick sources are scanned")]
fn the_kernel_and_tick_sources_are_scanned(_world: &mut World) {
    import_boundary();
}

#[when(expr = "a queued wait meets a wake and the seat drops")]
fn a_queued_wait_meets_a_wake(_world: &mut World) {
    queued_line_survives_a_dropped_seat();
}

#[when(expr = "the player drops and sits again with a queued wait")]
fn the_player_drops_and_sits_again(_world: &mut World) {
    a_reseated_player_keeps_the_queued_wait();
}

#[when(expr = "a committed wait wakes early")]
fn a_committed_wait_wakes_early(_world: &mut World) {
    early_wake_does_not_tick_again();
}

#[when(expr = "a queued wait is due")]
fn a_queued_wait_is_due(_world: &mut World) {
    a_due_wake_applies_the_queued_wait();
}

#[when(expr = "effect rows and outcome lists are played")]
fn effect_rows_and_outcome_lists(_world: &mut World) {
    effects_run_on_both_outcomes_and_lists_run_on_one();
}

#[when(expr = "an opposing total is in the queue")]
fn an_opposing_total_is_in_the_queue(_world: &mut World) {
    opposing_total_reused();
}

#[when(expr = "Rowan marks Nell while holding the knife")]
fn rowan_marks_nell_while_holding_the_knife(_world: &mut World) {
    a_held_knife_still_runs_the_opposed_row();
}

#[when(expr = "the crate manifests are scanned")]
fn the_crate_manifests_are_scanned(_world: &mut World) {
    manifests_do_not_name_a_model_crate();
}

#[when(expr = "the scheduler source is scanned")]
fn the_scheduler_source_is_scanned(_world: &mut World) {
    the_scheduler_does_not_spawn_a_tick();
}
