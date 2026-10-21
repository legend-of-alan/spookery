use cucumber::{then, when};

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use spookery::host::{
    call_gm_tool, commit_queued, handle_player_move, on_scheduled_wake, queue_wait,
    release_player_seat, seat_player, MoveOutcome, SocketEnd,
};
use spookery_kernel::api::apply::apply_effect;
use spookery_kernel::api::check::Outcome;
use spookery_kernel::api::draft::Draft;
use spookery_kernel::api::effect::{Effect, EffectKind};
use spookery_kernel::api::intent::{ActorKind, Intent, IntentArgs, Source, Target, ViewerId};
use spookery_kernel::api::meter::MeterLive;
use spookery_kernel::api::model::{
    ActCounters, ActorLive, Message, Now, PauseView, PlaceLive, Presence, RunState,
};
use spookery_kernel::api::rules::MoveReport;
use spookery_kernel::api::store::codec::emit_save;
use spookery_round::{catch_up, play, play_pack_once};

use crate::support;
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

fn open(pack: &Path) -> spookery_kernel::api::world::World {
    spookery_kernel::api::World::open(pack).unwrap_or_else(|err| panic!("World::open: {err}"))
}

fn now_of(pack: &Path) -> Now {
    spookery_round::resume(pack)
        .unwrap_or_else(|err| panic!("resume: {err}"))
        .now()
        .clone()
}

fn write_text(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .unwrap_or_else(|err| panic!("mkdir {}: {err}", parent.display()));
    }
    fs::write(path, text).unwrap_or_else(|err| panic!("write {}: {err}", path.display()));
}

fn stamp_pack_rev(pack: &Path) {
    let now_path = pack.join("state/save.yaml");
    let mut now =
        spookery_kernel::api::store::codec::load_save(&fs::read_to_string(&now_path).unwrap())
            .unwrap();
    now.pack_rev = spookery_kernel::api::canon::pack_rev(pack).unwrap();
    fs::write(&now_path, emit_save(&now).unwrap()).unwrap();
}

fn lamp_oil(now: &Now) -> u32 {
    now.places
        .get("docks_shop")
        .and_then(|place| place.items.get("lamp_oil"))
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
    let text = spookery_kernel::api::journal::read_record_line(pack, name)
        .unwrap_or_else(|err| panic!("read {name}: {}", err.detail));
    serde_json::from_str(&text).unwrap_or_else(|err| panic!("json {name}: {err}"))
}

fn list_tick_keys(pack: &Path) -> Vec<String> {
    spookery_kernel::api::journal::list_tick_keys(pack)
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
    let pack = support::temp_pack("self-win");
    for tick in 1..=12 {
        play_pack_once(&pack).unwrap_or_else(|err| panic!("tick {tick}: {err}"));
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

fn seat_rowan(world: &mut spookery_kernel::api::world::World) {
    world.set_presence(spookery_kernel::api::model::Presence {
        humans: 1,
        actors_present: vec!["pc_rowan".to_string()],
    });
}

fn save_now(pack: &Path, now: &Now) {
    fs::write(pack.join("state/save.yaml"), emit_save(now).unwrap()).unwrap();
}

fn evening_note() {
    let pack = support::temp_pack("evening");
    play_pack_once(&pack).unwrap_or_else(|err| panic!("round: {err}"));
    let now = now_of(&pack);
    assert_eq!(lamp_oil(&now), 4);
    assert_eq!(now.actors.get("mira").unwrap().place(), "mira_rooms");
    assert_eq!(now.messages.len(), 1);
    assert_eq!(now.messages[0].id, "note_mira_door");
    assert_eq!(now.messages[0].place.as_deref(), Some("docks_shop"));
    let _ = fs::remove_dir_all(&pack);
}

fn morning_restock() {
    let pack = support::temp_pack("morning");
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
    play_pack_once(&pack).unwrap_or_else(|err| panic!("round: {err}"));
    let now = now_of(&pack);
    assert_eq!(lamp_oil(&now), 8);
    assert_eq!(now.actors.get("mira").unwrap().place(), "mira_rooms");
    assert_eq!(now.messages.len(), 1);
    let _ = fs::remove_dir_all(&pack);
}

fn capped_restock() -> u32 {
    let pack = support::temp_pack("capped");
    {
        let shard = open(&pack);
        let mut now = shard.now().clone();
        drop(shard);
        now.world_time = 600;
        now.messages.push(door_message());
        *now.actors.get_mut("pc_rowan").unwrap().place_mut() = "docks_street".to_string();
        now.places
            .get_mut("docks_shop")
            .unwrap()
            .items
            .insert("lamp_oil".to_string(), 6);
        save_now(&pack, &now);
    }
    play_pack_once(&pack).unwrap_or_else(|err| panic!("round: {err}"));
    let oil = lamp_oil(&now_of(&pack));
    let _ = fs::remove_dir_all(&pack);
    oil
}

fn off_hours_hold() {
    let pack = support::temp_pack("off-hours");
    {
        let shard = open(&pack);
        let mut now = shard.now().clone();
        drop(shard);
        now.messages.push(door_message());
        save_now(&pack, &now);
    }
    play_pack_once(&pack).unwrap_or_else(|err| panic!("round: {err}"));
    let now = now_of(&pack);
    assert_eq!(lamp_oil(&now), 4);
    assert_eq!(now.messages.len(), 1);
    let _ = fs::remove_dir_all(&pack);
}

fn player_in_the_shop() {
    let pack = support::temp_pack("present");
    {
        let shard = open(&pack);
        let mut now = shard.now().clone();
        drop(shard);
        now.world_time = 600;
        now.messages.push(door_message());
        *now.actors.get_mut("mira").unwrap().place_mut() = "docks_shop".to_string();
        save_now(&pack, &now);
    }
    let mut shard = open(&pack);
    seat_rowan(&mut shard);
    let wall = shard.now().last_tick_at.clone();
    play(&mut shard, spookery_round::RoundCause::Manual, &wall)
        .unwrap_or_else(|err| panic!("round: {err}"));
    let now = now_of(&pack);
    assert_eq!(lamp_oil(&now), 4);
    assert_eq!(now.messages.len(), 1);
    assert_eq!(now.actors.get("mira").unwrap().act().last_act_tick, 0);
    let _ = fs::remove_dir_all(&pack);
}

fn gm_restocks() {
    let pack = support::temp_pack("gm-restock");
    {
        let shard = open(&pack);
        let mut now = shard.now().clone();
        drop(shard);
        now.world_time = 600;
        now.messages.push(door_message());
        *now.actors.get_mut("mira").unwrap().place_mut() = "docks_shop".to_string();
        save_now(&pack, &now);
    }
    let mut shard = open(&pack);
    seat_rowan(&mut shard);
    let wall = shard.now().last_tick_at.clone();
    let args = json!({
        "actor_id": "mira",
        "actor_kind": "npc",
        "source": "player",
        "move": "restock_lamp_oil"
    });
    let queued = call_gm_tool(
        &mut shard,
        "intent",
        Some(args.as_object().unwrap().clone()),
    );
    assert!(queued.value.get("code").is_none(), "{}", queued.value);
    play(&mut shard, spookery_round::RoundCause::Manual, &wall).unwrap();
    assert_eq!(lamp_oil(shard.now()), 8);
    assert_eq!(shard.now().messages.len(), 1);
    let _ = fs::remove_dir_all(&pack);
}

fn claimed_note_sorts_first() {
    let pack = support::temp_pack("claimed");
    {
        let shard = open(&pack);
        let mut now = shard.now().clone();
        drop(shard);
        *now.actors.get_mut("mira").unwrap().place_mut() = "docks_shop".to_string();
        save_now(&pack, &now);
    }
    let mut shard = open(&pack);
    seat_rowan(&mut shard);
    let wall = shard.now().last_tick_at.clone();
    for (actor, move_name) in [("bram", "wait"), ("mira", "leave_door_note")] {
        let args = json!({
            "actor_id": actor,
            "actor_kind": "npc",
            "source": "player",
            "move": move_name
        });
        let queued = call_gm_tool(
            &mut shard,
            "intent",
            Some(args.as_object().unwrap().clone()),
        );
        assert!(queued.value.get("code").is_none(), "{}", queued.value);
    }
    play(&mut shard, spookery_round::RoundCause::Manual, &wall).unwrap();
    let log = tick_value(&pack, "tick-0001");
    let moves: Vec<&str> = log["intents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|intent| intent["move"].as_str().unwrap())
        .collect();
    assert_eq!(moves, vec!["leave_door_note", "wait"]);
    assert_eq!(
        log["intents"][0]["args"]["to"][0].as_str(),
        Some("pc_rowan")
    );
    let _ = fs::remove_dir_all(&pack);
}

fn catchup_cap() {
    catchup_for(72, 12);
}

fn catchup_for(hours: u64, ticks: u64) {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let pack = support::fresh_dir("catchup");
    let _ = fs::remove_dir_all(&pack);
    let rules = manifest.join("../../packs/oneshot-docks/rules");
    write_text(
        &pack.join("pack.yaml"),
        "\
schema: pack
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
schema: place
id: gate
display_name: Gate
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
schema: cast
id: pc_rowan
kind: pc_template
display_name: Rowan
facts: []
---
Rowan.
",
    );
    for name in ["verbs.yaml", "checks.yaml", "meters.yaml", "tick.yaml"] {
        fs::copy(rules.join(name), {
            let dest = pack.join("rules").join(name);
            fs::create_dir_all(dest.parent().unwrap()).unwrap();
            dest
        })
        .unwrap_or_else(|err| panic!("copy {name}: {err}"));
    }
    let rev = spookery_kernel::api::canon::pack_rev(&pack).unwrap();
    let mut places = BTreeMap::new();
    places.insert(
        "gate".to_string(),
        PlaceLive {
            items: BTreeMap::new(),
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

            meters: meters(1),
            inventory: BTreeMap::new(),
            wielded: None,
            known_facts: Vec::new(),
            visited: vec!["gate".to_string()],
            act: ActCounters::fresh(),
        },
    );
    let now = Now {
        schema: "now".to_string(),
        shard_id: "ex".to_string(),
        pack_rev: rev,
        rules_rev: spookery_kernel::api::store::RULES_REV,
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
        messages: Vec::new(),
        flags: Vec::new(),
    };
    write_text(&pack.join("state/save.yaml"), &emit_save(&now).unwrap());
    let place_before = fs::read(pack.join("world/places/gate.md")).unwrap();
    let cast_before = fs::read(pack.join("cast/pc_rowan.md")).unwrap();
    let mut shard = open(&pack);
    let wall = shard.now().last_tick_at.clone();
    catch_up(&mut shard, &wall, hours).unwrap_or_else(|err| panic!("catch-up: {err}"));
    let saved = spookery_round::resume(&pack).unwrap_or_else(|err| panic!("resume: {err}"));
    let now = saved.now();
    assert_eq!(now.state_rev, ticks, "{hours} hours");
    assert_eq!(now.tick_index, ticks);
    assert_eq!(now.world_time, ticks * 60);
    assert_eq!(now.last_tick_at, "2026-01-01T00:00:00Z");
    assert_eq!(list_tick_keys(&pack).len() as u64, ticks);
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
    let pack = support::temp_pack("busy");
    {
        let shard = open(&pack);
        let mut now = shard.now().clone();
        drop(shard);
        now.world_time = 600;
        now.messages.push(door_message());
        now.actors.get_mut("mira").unwrap().act_mut().busy_until = Some(u64::MAX);
        *now.actors.get_mut("pc_rowan").unwrap().place_mut() = "docks_street".to_string();
        save_now(&pack, &now);
    }
    play_pack_once(&pack).unwrap_or_else(|err| panic!("round: {err}"));
    let now = now_of(&pack);
    assert_eq!(lamp_oil(&now), 4);
    assert_eq!(now.actors.get("mira").unwrap().act().last_act_tick, 0);
    assert_eq!(now.actors.get("bram").unwrap().act().last_act_tick, 0);
    let _ = fs::remove_dir_all(&pack);
}

fn mira_intent() -> Intent {
    Intent {
        id: "drop-txn".to_string(),
        tick_index: 1,
        actor_id: "mira".to_string(),
        actor_kind: ActorKind::Npc,
        source: Source::Kernel,
        move_name: "restock_lamp_oil".to_string(),
        target: Target::Place("docks_shop".to_string()),
        args: IntentArgs::default(),
        rationale_code: String::new(),
    }
}

fn stock_effect(delta: i64) -> Effect {
    Effect {
        class: EffectKind::Item {
            actor_id: None,
            place: Some("docks_shop".to_string()),
            id: "lamp_oil".to_string(),
            delta,
            max: None,
        },
        as_viewer: ViewerId::Npc("mira".to_string()),
        intent_id: "drop-txn".to_string(),
    }
}

fn sealed_tick(pack: &Path, state_rev: u64) -> String {
    use spookery_kernel::api::journal::{genesis_hash, seal, JournalEntry, TickRecord};
    let entry = JournalEntry::Tick(TickRecord {
        tick_index: 1,
        rules_rev: 7,
        state_rev_before: state_rev.saturating_sub(1),
        state_rev_after: state_rev,
        world_time_before: 0,
        world_time_after: 0,
        presence_humans: 0,
        actors_present: Vec::new(),
        seed: String::new(),
        last_tick_at: String::new(),
        intents: Vec::new(),
        rejected: Vec::new(),
        effects: Vec::new(),
        arrivals: Vec::new(),
        fast_forward: false,
        dice: Vec::new(),
        steps_run: Vec::new(),
        memories: vec![spookery_kernel::api::model::MemoryLine {
            tick_index: 1,
            actor_id: "mira".to_string(),
            intent_id: String::new(),
            fact_ids: Vec::new(),
            tags: Vec::new(),
            text: Some("kept-line".to_string()),
        }],
        now_hash: String::new(),
        prev: String::new(),
        chain: String::new(),
    });
    let prev = genesis_hash(pack).unwrap_or_else(|err| panic!("{}", err.detail));
    seal(&entry, &prev)
}

fn an_open_transaction_writes_nothing() {
    let dropped = support::temp_pack("drop");
    let now_path = dropped.join("state/save.yaml");
    let before = fs::read(&now_path).unwrap();
    let shard = open(&dropped);
    let state_rev = shard.now().state_rev;
    let oil = lamp_oil(shard.now());
    let delta = if oil > 0 { -1 } else { 1 };
    let expected = if oil > 0 { oil - 1 } else { oil + 1 };
    let mut draft = Draft::begin(&shard);
    apply_effect(&mut draft, &mira_intent(), stock_effect(delta)).unwrap();
    assert_eq!(lamp_oil(draft.now()), expected);
    drop(draft);
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
    let incomplete = support::temp_pack("incomplete");
    let now_path = incomplete.join("state/save.yaml");
    let before = fs::read(&now_path).unwrap();
    let txn = incomplete.join("state/.txn");
    fs::create_dir_all(txn.join("prev")).unwrap();
    fs::write(txn.join("prev/save.yaml"), b"not-yaml\n").unwrap();
    let opened = open(&incomplete);
    assert_eq!(fs::read(&now_path).unwrap(), before);
    assert_eq!(
        lamp_oil(opened.now()),
        lamp_oil(
            &spookery_kernel::api::store::codec::load_save(std::str::from_utf8(&before).unwrap())
                .unwrap()
        )
    );
    assert!(!txn.exists());
    let _ = fs::remove_dir_all(&incomplete);
}

fn the_same_revision_restores_the_previous_save() {
    let same = support::temp_pack("same-rev");
    let now_path = same.join("state/save.yaml");
    let original = fs::read(&now_path).unwrap();
    let shard = open(&same);
    let mut dirty = shard.now().clone();
    drop(shard);
    dirty
        .places
        .get_mut("docks_shop")
        .unwrap()
        .items
        .insert("lamp_oil".to_string(), 999);
    let dirty_bytes = emit_save(&dirty).unwrap();
    fs::write(&now_path, &dirty_bytes).unwrap();
    let txn = same.join("state/.txn");
    fs::create_dir_all(txn.join("prev")).unwrap();
    fs::write(txn.join("prev/save.yaml"), &original).unwrap();
    fs::write(txn.join("COMPLETE"), b"").unwrap();
    let opened = open(&same);
    assert_eq!(fs::read(&now_path).unwrap(), dirty_bytes.as_bytes());
    assert_eq!(lamp_oil(opened.now()), 999);
    assert!(!txn.exists());
    let _ = fs::remove_dir_all(&same);
}

fn the_next_revision_keeps_the_new_save() {
    let next = support::temp_pack("next-rev");
    let shard = open(&next);
    let prev = shard.now().clone();
    let mut live = prev.clone();
    drop(shard);
    live.state_rev = prev.state_rev.saturating_add(1);
    let kept_oil = lamp_oil(&prev).saturating_add(1);
    live.places
        .get_mut("docks_shop")
        .unwrap()
        .items
        .insert("lamp_oil".to_string(), kept_oil);
    fs::write(next.join("state/save.yaml"), emit_save(&live).unwrap()).unwrap();
    let txn = next.join("state/.txn");
    fs::create_dir_all(txn.join("prev")).unwrap();
    fs::write(txn.join("prev/save.yaml"), emit_save(&prev).unwrap()).unwrap();
    fs::write(txn.join("COMPLETE"), b"").unwrap();
    let log = next.join("state/journal");
    fs::create_dir_all(&log).unwrap();
    let kept = sealed_tick(&next, live.state_rev);
    let mut torn = kept.into_bytes();
    torn.extend(br#"{"tick_index":99,"#);
    let bundle = log.join("bundle-0000.jsonl");
    fs::write(&bundle, &torn).unwrap();
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
    let bundle_text = fs::read_to_string(&bundle).unwrap();
    assert!(bundle_text.ends_with('\n'), "{bundle_text}");
    assert!(!bundle_text.contains("tick_index\":99"));
    let memory = fs::read_to_string(memories.join("mira.jsonl")).unwrap();
    assert!(memory.contains("kept-line"));
    assert!(!memory.contains("orphan-line"));
    let _ = fs::remove_dir_all(&next);
}

fn budget_caps() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let pack = support::fresh_dir("budget");
    let _ = fs::remove_dir_all(&pack);
    let rules = manifest.join("../../packs/oneshot-docks/rules");
    write_text(
        &pack.join("pack.yaml"),
        "\
schema: pack
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
schema: place
id: yard
display_name: Yard
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
schema: cast
id: {id}
kind: npc
display_name: {id}
facts: []
tick:
  eligible: true
rules:
  - id: hold
    when: habit
    do: []
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

                meters: meters(1),
                inventory: BTreeMap::new(),
                wielded: None,
                known_facts: Vec::new(),
                act: ActCounters::fresh(),
            },
        );
    }
    for name in ["verbs.yaml", "checks.yaml", "meters.yaml", "tick.yaml"] {
        let dest = pack.join("rules").join(name);
        fs::create_dir_all(dest.parent().unwrap()).unwrap();
        fs::copy(rules.join(name), &dest).unwrap();
    }
    let rev = spookery_kernel::api::canon::pack_rev(&pack).unwrap();
    let mut places = BTreeMap::new();
    places.insert(
        "yard".to_string(),
        PlaceLive {
            items: BTreeMap::new(),
            notes: Vec::new(),
            actors: Vec::new(),
        },
    );
    let now = Now {
        schema: "now".to_string(),
        shard_id: "crowd".to_string(),
        pack_rev: rev,
        rules_rev: spookery_kernel::api::store::RULES_REV,
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
        messages: Vec::new(),
        flags: Vec::new(),
    };
    write_text(&pack.join("state/save.yaml"), &emit_save(&now).unwrap());
    play_pack_once(&pack).unwrap_or_else(|err| panic!("round: {err}"));
    let now = now_of(&pack);
    assert_eq!(now.tick_index, 1);
    assert_eq!(
        now.actors.values().filter(|actor| actor.is_npc()).count(),
        50
    );
    let acted = now
        .actors
        .values()
        .filter(|actor| actor.act().last_act_tick == 1)
        .count();
    let applied = tick_value(&pack, "tick-0001")["intents"]
        .as_array()
        .unwrap()
        .len();
    assert!((1..=8).contains(&acted), "acted {acted}");
    assert!((1..=12).contains(&applied), "applied {applied}");
    let _ = fs::remove_dir_all(&pack);
}

fn tool_scope() {
    let pack = support::temp_pack("scope");
    let before = snapshot(&pack);
    let shard = open(&pack);
    let call = |as_viewer: &str, budget: usize| {
        spookery_kernel::api::perceive::perceive_budget(&shard, as_viewer, budget)
    };
    let kernel = spookery_kernel::api::perceive(&shard, "npc:mira").unwrap();
    assert_eq!(call("npc:mira", 800).unwrap(), kernel);
    let capped = call("npc:mira", 0).unwrap();
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
    let unscoped = spookery_kernel::api::Reason::UnscopedPerceive;
    assert_eq!(call("gm", 800).unwrap_err(), unscoped);
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
    let pack = support::temp_pack("strip");
    let mut files = Vec::new();
    markdown_files(&pack.join("cast"), &mut files);
    markdown_files(&pack.join("world"), &mut files);
    assert!(!files.is_empty());
    for path in &files {
        let text = fs::read_to_string(path).unwrap();
        let stripped = spookery_kernel::api::lint::strip_body(&text).unwrap();
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
    play_pack_once(&pack).unwrap_or_else(|err| panic!("stripped pack did not tick: {err}"));
    let perceive = format!(
        "{:?}",
        spookery_kernel::api::perceive(&open(&pack), "npc:aldric").unwrap()
    );
    assert!(!perceive.contains(LEDGER));

    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packs/oneshot-docks");
    assert!(spookery_kernel::api::lint::lint(&fixture)
        .unwrap()
        .is_empty());
    let owner = support::temp_pack("lint-owner");
    let mut body = fs::read_to_string(owner.join("cast/mira.md")).unwrap();
    body.push_str(LEDGER);
    body.push('\n');
    fs::write(owner.join("cast/mira.md"), body).unwrap();
    let hits = spookery_kernel::api::lint::lint(&owner).unwrap();
    assert!(hits
        .iter()
        .any(|hit| hit.path == "cast/mira.md" && hit.text == LEDGER));
    let place = support::temp_pack("lint-place");
    let mut body = fs::read_to_string(place.join("world/places/docks_shop.md")).unwrap();
    body.push_str(LEDGER);
    body.push('\n');
    fs::write(place.join("world/places/docks_shop.md"), body).unwrap();
    let hits = spookery_kernel::api::lint::lint(&place).unwrap();
    assert!(hits
        .iter()
        .any(|hit| hit.path == "world/places/docks_shop.md" && hit.text == LEDGER));
    let folded = support::temp_pack("lint-case");
    let mut body = fs::read_to_string(folded.join("cast/mira.md")).unwrap();
    body.push_str("the lamp-oil ledger is short.\n");
    fs::write(folded.join("cast/mira.md"), body).unwrap();
    assert!(spookery_kernel::api::lint::lint(&folded)
        .unwrap()
        .is_empty());
    let _ = fs::remove_dir_all(&pack);
    let _ = fs::remove_dir_all(&owner);
    let _ = fs::remove_dir_all(&place);
    let _ = fs::remove_dir_all(&folded);
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

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

fn byte_hits(paths: &[PathBuf], needles: &[&str]) -> Vec<String> {
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
}

fn hits_under(dir: &Path, needles: &[&str]) -> Vec<String> {
    let mut files = Vec::new();
    files_under(dir, &mut files);
    byte_hits(&files, needles)
}

const KERNEL_TICK_NEEDLES: &[&str] = &[
    "tokio",
    "rmcp",
    "sqlx",
    "postgres",
    "redis",
    "reqwest",
    "langchain",
    "openai",
    "anthropic",
    "ollama",
];

fn import_boundary() {
    let root = workspace_root();
    let spook_src = root.join("crates/spook/src");
    let mut kernel_hits = hits_under(&root.join("crates/kernel"), KERNEL_TICK_NEEDLES);
    kernel_hits.extend(hits_under(&root.join("crates/round"), KERNEL_TICK_NEEDLES));
    assert!(kernel_hits.is_empty(), "{}", kernel_hits.join("\n"));

    let mut runtime = Vec::new();
    files_under(&root.join("crates/kernel"), &mut runtime);
    files_under(&root.join("crates/round"), &mut runtime);
    files_under(&spook_src, &mut runtime);
    let ai = spook_src.join("ai");
    let mut reqwest_hits = Vec::new();
    for path in &runtime {
        if path.starts_with(&ai) || path.ends_with("gm_connection.rs") {
            continue;
        }
        let bytes = fs::read(path).unwrap();
        let needle = b"reqwest";
        if bytes.windows(needle.len()).any(|window| window == needle) {
            reqwest_hits.push(format!("{} contains reqwest", path.display()));
        }
    }
    assert!(reqwest_hits.is_empty(), "{}", reqwest_hits.join("\n"));
    let sdk_hits = byte_hits(&runtime, &["langchain", "openai", "anthropic", "ollama"]);
    assert!(sdk_hits.is_empty(), "{}", sdk_hits.join("\n"));

    let protocol_hits = hits_under(
        &spook_src.join("protocol"),
        &[
            "tokio",
            "rmcp",
            "reqwest",
            "spookery_kernel",
            "crate::terminal",
            "crate::ai",
        ],
    );
    assert!(protocol_hits.is_empty(), "{}", protocol_hits.join("\n"));
    let terminal_hits = hits_under(
        &spook_src.join("terminal"),
        &["tokio", "rmcp", "reqwest", "crate::ai"],
    );
    assert!(terminal_hits.is_empty(), "{}", terminal_hits.join("\n"));
    let gm_connection_hits = byte_hits(
        &[spook_src.join("gm_connection.rs")],
        &["crate::ai", "crate::terminal"],
    );
    assert!(
        gm_connection_hits.is_empty(),
        "{}",
        gm_connection_hits.join("\n")
    );
    let ai_hits = hits_under(
        &spook_src.join("ai"),
        &["rmcp", "crate::terminal", "spookery_round"],
    );
    assert!(ai_hits.is_empty(), "{}", ai_hits.join("\n"));
    let host_hits = hits_under(&spook_src.join("host"), &["crate::terminal", "crate::ai"]);
    assert!(host_hits.is_empty(), "{}", host_hits.join("\n"));
    let audit_hits = byte_hits(&[spook_src.join("audit.rs")], &["tokio", "rmcp", "reqwest"]);
    assert!(audit_hits.is_empty(), "{}", audit_hits.join("\n"));
    crate::kernel_boundary::clean_tree_holds();
}

fn five_e_words_under(dir: &Path) -> Vec<String> {
    let mut files = Vec::new();
    files_under(dir, &mut files);
    let mut hits = Vec::new();
    for path in files {
        let bytes = fs::read(&path).unwrap();
        for word in [b"5e".as_slice(), b"d&d", b"srd"] {
            if contains_word(&bytes, word) {
                let label = String::from_utf8_lossy(word);
                hits.push(format!("{} contains {label}", path.display()));
            }
        }
    }
    hits
}

fn contains_word(hay: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || hay.len() < needle.len() {
        return false;
    }
    let last = hay.len() - needle.len();
    for index in 0..=last {
        if !hay[index..index + needle.len()].eq_ignore_ascii_case(needle) {
            continue;
        }
        let before = index == 0 || !is_word_byte(hay[index - 1]);
        let after_at = index + needle.len();
        let after = after_at == hay.len() || !is_word_byte(hay[after_at]);
        if before && after {
            return true;
        }
    }
    false
}

fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn no_five_e_words_in_kernel_or_tick() {
    let root = workspace_root();
    let mut hits = five_e_words_under(&root.join("crates/kernel"));
    hits.extend(five_e_words_under(&root.join("crates/round")));
    assert!(hits.is_empty(), "{}", hits.join("\n"));
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
    let held = support::temp_pack("held");
    let mut shard = open(&held);
    let session = seat_player(&mut shard, "pc_rowan").unwrap();
    assert!(!on_scheduled_wake(&mut shard, true, false).unwrap());
    let note = queue_wait(&mut shard, "pc_rowan").unwrap();
    assert!(!on_scheduled_wake(&mut shard, true, true).unwrap());
    assert_eq!(shard.queued_intents().len(), 1);
    assert_eq!(shard.queued_intents()[0].id, note.id);
    assert_eq!(shard.now().world_time, 1080);
    assert!(list_tick_keys(&held).is_empty());
    assert!(release_player_seat(&mut shard, &session, "pc_rowan", SocketEnd::Close).unwrap());
    assert_eq!(shard.now().presence.humans, 0);
    assert!(!on_scheduled_wake(&mut shard, false, true).unwrap());
    assert_eq!(shard.queued_intents().len(), 1);
    assert_eq!(shard.now().tick_index, 0);
    assert!(list_tick_keys(&held).is_empty());
    let _ = fs::remove_dir_all(&held);
}

fn a_reseated_player_keeps_the_queued_wait() {
    let dropped = support::temp_pack("drop-queue");
    let mut shard = open(&dropped);
    let session = seat_player(&mut shard, "pc_rowan").unwrap();
    let note = handle_player_move(&mut shard, "pc_rowan", "wait", true).unwrap();
    let MoveOutcome::Queued(note) = note else {
        panic!("queued wait committed");
    };
    assert!(release_player_seat(&mut shard, &session, "pc_rowan", SocketEnd::Close).unwrap());
    assert_eq!(shard.now().world_time, 1080);
    assert_eq!(shard.now().tick_index, 0);
    assert_eq!(shard.queued_intents().len(), 1);
    assert!(list_tick_keys(&dropped).is_empty());
    seat_player(&mut shard, "pc_rowan").unwrap();
    assert_eq!(shard.now().world_time, 1080);
    assert_eq!(shard.now().tick_index, 0);
    assert_eq!(shard.queued_intents()[0].id, note.id);
    assert!(list_tick_keys(&dropped).is_empty());
    let _ = fs::remove_dir_all(&dropped);
}

fn early_wake_does_not_tick_again() {
    let early = support::temp_pack("early-wake");
    let mut shard = open(&early);
    let session = seat_player(&mut shard, "pc_rowan").unwrap();
    queue_wait(&mut shard, "pc_rowan").unwrap();
    assert!(commit_queued(&mut shard).unwrap().0);
    let committed = shard.now().clone();
    assert_eq!(committed.world_time, 1085);
    assert!(release_player_seat(&mut shard, &session, "pc_rowan", SocketEnd::Close).unwrap());
    assert!(!on_scheduled_wake(&mut shard, false, false).unwrap());
    assert_eq!(shard.now().world_time, committed.world_time);
    assert_eq!(shard.now().tick_index, committed.tick_index);
    assert_eq!(list_tick_keys(&early), vec!["tick-0001".to_string()]);
    let _ = fs::remove_dir_all(&early);
}

fn write_idle(pack: &Path, value: &str) {
    let path = pack.join("rules/tick.yaml");
    let text = fs::read_to_string(&path).unwrap();
    assert!(!text.contains("\nidle:"), "tick.yaml already sets idle");
    let next = text.replacen(
        "schema: tick\n",
        &format!("schema: tick\nidle: {value}\n"),
        1,
    );
    assert_ne!(next, text, "tick.yaml has no schema line");
    fs::write(&path, next).unwrap();
    stamp_pack_rev(pack);
}

fn idle_off_past_due_wake_writes_nothing() {
    let pack = support::temp_pack("idle-off-due");
    write_idle(&pack, "off");
    let (world_time, state_rev, tick_index, stamp) = {
        let shard = open(&pack);
        let every = shard.budget().tick_every_real_seconds;
        assert!(
            every > 0 && every + 60 < 3600,
            "interval {every} is not inside one hour"
        );
        assert!(!shard.budget().idle_ticking);
        let stamp = spookery::host::idle_clock::wall_ago(every + 60).unwrap();
        let mut now = shard.now().clone();
        let world_time = now.world_time;
        let state_rev = now.state_rev;
        let tick_index = now.tick_index;
        drop(shard);
        now.last_tick_at = stamp.clone();
        save_now(&pack, &now);
        (world_time, state_rev, tick_index, stamp)
    };
    let mut shard = open(&pack);
    let elapsed = spookery::host::idle_clock::seconds_since(&stamp).unwrap();
    assert!(elapsed >= shard.budget().tick_every_real_seconds);
    assert!(elapsed < 3600);
    assert!(!on_scheduled_wake(&mut shard, false, false).unwrap());
    assert_eq!(shard.now().world_time, world_time);
    assert_eq!(shard.now().last_tick_at, stamp);
    assert_eq!(shard.now().state_rev, state_rev);
    assert_eq!(shard.now().tick_index, tick_index);
    drop(shard);
    let saved = now_of(&pack);
    assert_eq!(saved.world_time, world_time);
    assert_eq!(saved.last_tick_at, stamp);
    assert_eq!(saved.state_rev, state_rev);
    assert!(list_tick_keys(&pack).is_empty());
    let _ = fs::remove_dir_all(&pack);
}

fn idle_off_long_absence_writes_no_catchup() {
    let pack = support::temp_pack("idle-off-absence");
    write_idle(&pack, "off");
    let mut shard = open(&pack);
    let world_time = shard.now().world_time;
    let last = shard.now().last_tick_at.clone();
    let state_rev = shard.now().state_rev;
    let tick_index = shard.now().tick_index;
    assert!(spookery::host::idle_clock::seconds_since(&last).unwrap() >= 3600);
    assert!(!on_scheduled_wake(&mut shard, false, false).unwrap());
    assert_eq!(shard.now().world_time, world_time);
    assert_eq!(shard.now().last_tick_at, last);
    assert_eq!(shard.now().tick_index, tick_index);
    drop(shard);
    let saved = now_of(&pack);
    assert_eq!(saved.world_time, world_time);
    assert_eq!(saved.last_tick_at, last);
    assert_eq!(saved.state_rev, state_rev);
    assert!(list_tick_keys(&pack).is_empty());
    let _ = fs::remove_dir_all(&pack);
}

fn idle_off_seated_line_and_spook_tick_still_advance() {
    let pack = support::temp_pack("idle-off-play");
    write_idle(&pack, "off");
    let mut shard = open(&pack);
    seat_player(&mut shard, "pc_rowan").unwrap();
    let played = handle_player_move(&mut shard, "pc_rowan", "wait", false).unwrap();
    let MoveOutcome::Committed { advanced, .. } = played else {
        panic!("wait stayed queued");
    };
    assert!(advanced);
    assert_eq!(shard.now().world_time, 1085);
    assert_eq!(shard.now().tick_index, 1);
    drop(shard);
    assert_eq!(list_tick_keys(&pack), vec!["tick-0001".to_string()]);
    let mut again = spookery_round::resume(&pack).unwrap_or_else(|err| panic!("resume: {err}"));
    seat_player(&mut again, "pc_rowan").unwrap();
    let wall = again.now().last_tick_at.clone();
    play(&mut again, spookery_round::RoundCause::Manual, &wall)
        .unwrap_or_else(|err| panic!("round: {err}"));
    let now = now_of(&pack);
    assert_eq!(now.tick_index, 2);
    assert_eq!(now.world_time, 1090);
    assert_eq!(
        list_tick_keys(&pack),
        vec!["tick-0001".to_string(), "tick-0002".to_string()]
    );
    let _ = fs::remove_dir_all(&pack);
}

fn idle_off_commit_round_still_commits() {
    let pack = support::temp_pack("idle-off-commit");
    write_idle(&pack, "off");
    let mut shard = open(&pack);
    seat_player(&mut shard, "pc_rowan").unwrap();
    let queued = handle_player_move(&mut shard, "pc_rowan", "wait", true).unwrap();
    let MoveOutcome::Queued(_) = queued else {
        panic!("wait committed with a GM seated");
    };
    assert_eq!(shard.now().world_time, 1080);
    assert!(list_tick_keys(&pack).is_empty());
    let done = call_gm_tool(
        &mut shard,
        "commit_round",
        Some(json!({ "include_tick": true }).as_object().unwrap().clone()),
    );
    assert!(done.value.get("code").is_none(), "{}", done.value);
    assert!(done.ticked);
    assert_eq!(shard.now().world_time, 1085);
    assert_eq!(shard.now().tick_index, 1);
    assert_eq!(list_tick_keys(&pack), vec!["tick-0001".to_string()]);
    let _ = fs::remove_dir_all(&pack);
}

fn a_due_wake_applies_the_queued_wait() {
    let due = support::temp_pack("due-wake");
    let mut shard = open(&due);
    let note = queue_wait(&mut shard, "pc_rowan").unwrap();
    assert_eq!(shard.now().last_tick_at, "2026-01-01T00:00:00Z");
    assert!(on_scheduled_wake(&mut shard, false, false).unwrap());
    assert_eq!(shard.now().tick_index, 12);
    assert_eq!(shard.now().world_time, 1800);
    assert!(shard.queued_intents().is_empty());
    assert_eq!(player_waits(&tick_value(&due, "tick-0001")), vec![note.id]);
    assert!(player_waits(&tick_value(&due, "tick-0002")).is_empty());
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
    super::common::edit_save(pack, |now| support::place_of(now, "mira", "docks_shop"));
}

fn play_mark(pack: &Path, line: &str) -> Now {
    let mut shard = open(pack);
    seat_player(&mut shard, "pc_rowan").unwrap();
    let played = handle_player_move(&mut shard, "pc_rowan", line, false).unwrap();
    let MoveOutcome::Committed { .. } = played else {
        panic!("{line} stayed queued");
    };
    shard.now().clone()
}

fn effects_run_on_both_outcomes_and_lists_run_on_one() {
    for (label, difficulty, success) in [("effects-pass", 0, true), ("effects-fail", 14, false)] {
        let pack = support::temp_pack(label);
        let mut moves = fs::read_to_string(pack.join("rules/verbs.yaml")).unwrap();
        moves.push_str(&format!(
            "\n  - id: mark\n    when:\n      verb: mark\n      target: actor\n    roll:\n      check: grit\n      target: {difficulty}\n    do:\n      - flag:\n          who: target\n          key: marked\n          value: true\n",
        ));
        fs::write(pack.join("rules/verbs.yaml"), moves).unwrap();
        patch_mira_flags(&pack, "    - marked\n");
        stamp_pack_rev(&pack);
        let now = play_mark(&pack, "mark mira");
        assert_eq!(now.world_time, 1085);
        assert_eq!(
            flags_of(&now, "mira"),
            vec!["marked".to_string()],
            "{label} success {success}"
        );
        let tick = tick_value(&pack, "tick-0001");
        assert!(tick["steps_run"]
            .as_array()
            .unwrap()
            .iter()
            .any(|name| name == "resolve_check"));
        assert!(
            !spookery_kernel::api::journal::read_record_line(&pack, "tick-0001")
                .unwrap_or_else(|err| panic!("{}", err.detail))
                .contains("faces")
        );
        let _ = fs::remove_dir_all(&pack);
    }

    for (label, difficulty, flag) in [("lists-pass", 0, "marked"), ("lists-fail", 14, "missed")] {
        let pack = support::temp_pack(label);
        let mut moves = fs::read_to_string(pack.join("rules/verbs.yaml")).unwrap();
        moves.push_str(&format!(
            "\n  - id: mark\n    when:\n      verb: mark\n      target: actor\n    roll:\n      check: grit\n      target: {difficulty}\n    success:\n      - flag:\n          who: target\n          key: marked\n          value: true\n    failure:\n      - flag:\n          who: target\n          key: missed\n          value: true\n",
        ));
        fs::write(pack.join("rules/verbs.yaml"), moves).unwrap();
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
schema: cast
id: pc_nell
kind: pc_template
display_name: Nell
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
    let now_path = pack.join("state/save.yaml");
    let mut now =
        spookery_kernel::api::store::codec::load_save(&fs::read_to_string(&now_path).unwrap())
            .unwrap();
    now.actors.insert(
        "pc_nell".to_string(),
        ActorLive::Pc {
            place: "docks_shop".to_string(),
            flags: Vec::new(),

            meters: meters(100),
            inventory: BTreeMap::new(),
            wielded: None,
            known_facts: Vec::new(),
            visited: vec!["docks_shop".to_string()],
            act: ActCounters::fresh(),
        },
    );
    fs::write(&now_path, emit_save(&now).unwrap()).unwrap();
}

fn rolled_stat(reports: &[MoveReport], stat: i64) -> spookery_kernel::api::rules::RollResult {
    reports
        .iter()
        .find_map(|report| match report {
            MoveReport::Rolled { out, .. } if out.stat == stat => Some(out.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no stat {stat} in {reports:?}"))
}

fn opposing_total_reused() {
    for opposed in [false, true] {
        let label = if opposed { "opposed" } else { "open-check" };
        let pack = support::temp_pack(label);
        let mut moves = fs::read_to_string(pack.join("rules/verbs.yaml")).unwrap();
        moves.push_str(
            "\n  - id: mark\n    when:\n      verb: mark\n      target: actor\n    roll:\n      check: grit\n      target: 0\n    success:\n      - flag:\n          who: target\n          key: marked\n          value: true\n    failure:\n      - flag:\n          who: target\n          key: missed\n          value: true\n  - id: defend\n    when:\n      verb: defend\n    roll:\n      check: grit\n      target: 0\n    success:\n      - flag:\n          who: self\n          key: guarded\n          value: true\n",
        );
        fs::write(pack.join("rules/verbs.yaml"), moves).unwrap();
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
            let queued = call_gm_tool(
                &mut shard,
                "intent",
                Some(args.as_object().unwrap().clone()),
            );
            assert!(queued.value.get("code").is_none(), "{}", queued.value);
        }
        let mark = handle_player_move(&mut shard, "pc_rowan", "mark pc_nell", true).unwrap();
        let MoveOutcome::Queued(_) = mark else {
            panic!("mark committed while a gm seat was requested");
        };
        assert_eq!(shard.now().world_time, 1080);
        let (advanced, output) = commit_queued(&mut shard).unwrap();
        assert!(advanced);
        let reports = output.reports;
        let rowan = rolled_stat(&reports, 1);
        assert_eq!(shard.now().world_time, 1085);
        let tick = spookery_kernel::api::journal::read_record_line(&pack, "tick-0001")
            .unwrap_or_else(|err| panic!("{}", err.detail));
        assert!(!tick.contains("faces"));
        if opposed {
            let nell = rolled_stat(&reports, 100);
            assert_eq!(nell.target, 0);
            assert_eq!(nell.outcome, Outcome::Success);
            assert!(nell.total >= 102, "{nell:?}");
            assert_eq!(rowan.target, nell.total);
            assert_eq!(rowan.outcome, Outcome::Miss);
            let mut flags = flags_of(shard.now(), "pc_nell");
            flags.sort();
            assert_eq!(flags, vec!["guarded".to_string(), "missed".to_string()]);
        } else {
            assert_eq!(rowan.target, 0);
            assert_eq!(rowan.outcome, Outcome::Success);
            assert_eq!(flags_of(shard.now(), "pc_nell"), vec!["marked".to_string()]);
        }
        let _ = fs::remove_dir_all(&pack);
    }
}

fn a_held_knife_still_runs_the_opposed_row() {
    let held = support::temp_pack("item-held");
    let mut moves = fs::read_to_string(held.join("rules/verbs.yaml")).unwrap();
    moves.push_str(
        "\n  - id: mark\n    when:\n      verb: mark\n      target: actor\n    if:\n      - has_item:\n          who: self\n          id: knife\n    roll:\n      check: grit\n      target: 0\n    success:\n      - flag:\n          who: target\n          key: marked\n          value: true\n    failure:\n      - flag:\n          who: target\n          key: missed\n          value: true\n  - id: defend\n    when:\n      verb: defend\n    roll:\n      check: grit\n      target: 0\n    success:\n      - flag:\n          who: self\n          key: guarded\n          value: true\n",
    );
    fs::write(held.join("rules/verbs.yaml"), moves).unwrap();
    let knife = held.join("world/items/knife.md");
    fs::create_dir_all(knife.parent().unwrap()).unwrap();
    fs::write(
        &knife,
        "---\nschema: item\nid: knife\nname: Knife\ntags: []\nrules: []\n---\n\nA knife.\n",
    )
    .unwrap();
    insert_nell(&held);
    let now_path = held.join("state/save.yaml");
    let text = fs::read_to_string(&now_path).unwrap();
    let next = text.replacen(
        "  pc_rowan:\n    kind: pc\n    place: docks_shop\n    flags: []\n    meters:\n      grit:\n        base: 1\n        offset: 0\n      rest:\n        base: 3\n        offset: 0\n    inventory: {}\n",
        "  pc_rowan:\n    kind: pc\n    place: docks_shop\n    flags: []\n    meters:\n      grit:\n        base: 1\n        offset: 0\n      rest:\n        base: 3\n        offset: 0\n    inventory:\n      knife: 1\n",
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
    call_gm_tool(
        &mut shard,
        "intent",
        Some(args.as_object().unwrap().clone()),
    );
    let queued = handle_player_move(&mut shard, "pc_rowan", "mark pc_nell", true).unwrap();
    let MoveOutcome::Queued(_) = queued else {
        panic!("held mark committed before the round");
    };
    let (advanced, output) = commit_queued(&mut shard).unwrap();
    assert!(advanced);
    let rowan = rolled_stat(&output.reports, 1);
    assert_eq!(rowan.outcome, Outcome::Miss);
    assert!(
        !spookery_kernel::api::journal::read_record_line(&held, "tick-0001")
            .unwrap_or_else(|err| panic!("{}", err.detail))
            .contains("faces")
    );
    let _ = fs::remove_dir_all(&held);
}

fn manifests_do_not_name_a_model_crate() {
    let root = workspace_root();
    for path in [
        root.join("Cargo.toml"),
        root.join("crates/kernel/Cargo.toml"),
        root.join("crates/round/Cargo.toml"),
        root.join("crates/spook/Cargo.toml"),
    ] {
        let text = fs::read_to_string(&path).unwrap();
        for needle in ["llm", "langchain", "openai", "anthropic", "ollama"] {
            assert!(
                !text.contains(needle),
                "{} contains {needle}",
                path.display()
            );
        }
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

#[when(expr = "the morning tick runs and the shelf already holds 6 lamp oil")]
fn the_morning_tick_caps_the_shelf(world: &mut World) {
    world.scratch = capped_restock().to_string();
}

#[then(expr = "the shop holds 8 lamp oil")]
fn the_shop_holds_8_lamp_oil(world: &mut World) {
    assert_eq!(world.scratch, "8");
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

#[when(expr = "one hour of catch-up runs")]
fn one_hour_of_catch_up(_world: &mut World) {
    catchup_for(1, 8);
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

#[when(expr = "the kernel and tick sources are scanned for a game-words ban")]
fn the_kernel_and_tick_sources_are_scanned_for_a_game_words_ban(_world: &mut World) {
    no_five_e_words_in_kernel_or_tick();
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

#[when(expr = "idle is off and a past-due wake arrives")]
fn idle_is_off_and_a_past_due_wake_arrives(_world: &mut World) {
    idle_off_past_due_wake_writes_nothing();
}

#[when(expr = "idle is off and a long absence wakes")]
fn idle_is_off_and_a_long_absence_wakes(_world: &mut World) {
    idle_off_long_absence_writes_no_catchup();
}

#[when(expr = "idle is off and a player waits and spook tick runs")]
fn idle_is_off_and_a_player_waits(_world: &mut World) {
    idle_off_seated_line_and_spook_tick_still_advance();
}

#[when(expr = "idle is off and the GM commits a queued wait")]
fn idle_is_off_and_the_gm_commits(_world: &mut World) {
    idle_off_commit_round_still_commits();
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
