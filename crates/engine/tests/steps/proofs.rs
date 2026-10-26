use cucumber::{then, when};

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use spookery_engine::testing::{
    self as support, insert_nell, journal::list_tick_keys, now_of, open, read_tick as tick_value,
    save_now, stamp_pack_rev, ActCounters, ActorLive, PauseView, PlaceLive, Presence,
};
use spookery_engine::{
    emit_save, handle_player_move, play_pack_once, GmIntentSpec, Message, MeterLive, MoveOutcome,
    MoveReport, Now, Outcome, RunState, World as Shard,
};

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

fn write_text(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .unwrap_or_else(|err| panic!("mkdir {}: {err}", parent.display()));
    }
    fs::write(path, text).unwrap_or_else(|err| panic!("write {}: {err}", path.display()));
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

fn seat_rowan(world: &mut Shard) {
    world.set_presence(spookery_engine::testing::Presence {
        humans: 1,
        actors_present: vec!["pc_rowan".to_string()],
    });
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
    shard
        .play(spookery_engine::RoundCause::Manual, &wall)
        .unwrap_or_else(|err| panic!("round: {err}"));
    let now = now_of(&pack);
    assert_eq!(lamp_oil(&now), 4);
    assert_eq!(now.messages.len(), 1);
    assert_eq!(now.actors.get("mira").unwrap().act().last_act_tick, 0);
    let _ = fs::remove_dir_all(&pack);
}

fn queue_gm(shard: &mut Shard, id: &str, actor: &str, move_name: &str) {
    shard
        .queue_gm_intent(
            GmIntentSpec {
                id: None,
                tick_index: None,
                actor_id: Some(actor.to_string()),
                move_name: Some(move_name.to_string()),
                target: None,
                message_to: None,
                rationale_code: String::new(),
            },
            || Ok(id.to_string()),
        )
        .unwrap();
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
    queue_gm(&mut shard, "gm-1", "mira", "restock_lamp_oil");
    shard
        .play(spookery_engine::RoundCause::Manual, &wall)
        .unwrap();
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
    for (id, actor, move_name) in [
        ("gm-1", "bram", "wait"),
        ("gm-2", "mira", "leave_door_note"),
    ] {
        queue_gm(&mut shard, id, actor, move_name);
    }
    shard
        .play(spookery_engine::RoundCause::Manual, &wall)
        .unwrap();
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
    let rev = spookery_engine::pack_rev(&pack).unwrap();
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
        rules_rev: spookery_engine::testing::RULES_REV,
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
    shard
        .catch_up(&wall, hours)
        .unwrap_or_else(|err| panic!("catch-up: {err}"));
    let saved = Shard::resume(&pack).unwrap_or_else(|err| panic!("resume: {err}"));
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

fn sealed_tick(pack: &Path, state_rev: u64) -> String {
    use spookery_engine::testing::journal::{genesis_hash, seal, JournalEntry, TickRecord};
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
        memories: vec![spookery_engine::testing::MemoryLine {
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
    let mut shard = open(&dropped);
    let state_rev = shard.now().state_rev;
    let oil = lamp_oil(shard.now());
    shard.seat_player("pc_rowan").unwrap();
    shard
        .queue_player_move("pc_rowan", "take", Some("lamp_oil"))
        .unwrap();
    assert_eq!(shard.queued_intents().len(), 1);
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
        lamp_oil(&spookery_engine::load_save(std::str::from_utf8(&before).unwrap()).unwrap())
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
    let rev = spookery_engine::pack_rev(&pack).unwrap();
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
        rules_rev: spookery_engine::testing::RULES_REV,
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
    let call = |as_viewer: &str, budget: usize| shard.perceive_budget(as_viewer, budget);
    let kernel = shard.perceive("mira").unwrap();
    assert_eq!(call("mira", 800).unwrap(), kernel);
    let capped = call("mira", 0).unwrap();
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
    let unscoped = spookery_engine::Reason::UnscopedPerceive;
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

fn strip_bodies() {
    let pack = support::temp_pack("strip");
    let mut files = Vec::new();
    markdown_files(&pack.join("cast"), &mut files);
    markdown_files(&pack.join("world"), &mut files);
    assert!(!files.is_empty());
    for path in &files {
        let text = fs::read_to_string(path).unwrap();
        let stripped = spookery_engine::testing::strip_body(&text).unwrap();
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
    let perceive = format!("{:?}", open(&pack).perceive("aldric").unwrap());
    assert!(!perceive.contains(LEDGER));
    let _ = fs::remove_dir_all(&pack);
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
    let now_path = pack.join("state/save.yaml");
    let mut now = spookery_engine::load_save(&fs::read_to_string(&now_path).unwrap()).unwrap();
    support::place_of(&mut now, "mira", "docks_shop");
    save_now(pack, &now);
}

fn play_mark(pack: &Path) -> Now {
    let mut shard = open(pack);
    shard.seat_player("pc_rowan").unwrap();
    let wall = shard.now().last_tick_at.clone();
    let played =
        handle_player_move(&mut shard, "pc_rowan", "mark", Some("mira"), false, &wall).unwrap();
    let MoveOutcome::Committed { .. } = played else {
        panic!("mark mira stayed queued");
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
        let now = play_mark(&pack);
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
            !spookery_engine::testing::journal::read_record_line(&pack, "tick-0001")
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
        let now = play_mark(&pack);
        assert_eq!(flags_of(&now, "mira"), vec![flag.to_string()], "{label}");
        assert_eq!(now.world_time, 1085);
        let _ = fs::remove_dir_all(&pack);
    }
}

fn rolled_stat(reports: &[MoveReport], stat: i64) -> spookery_engine::RollResult {
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
        shard.seat_player("pc_rowan").unwrap();
        if opposed {
            queue_gm(&mut shard, "gm-1", "pc_nell", "defend");
        }
        let wall = shard.now().last_tick_at.clone();
        let mark = handle_player_move(&mut shard, "pc_rowan", "mark", Some("pc_nell"), true, &wall)
            .unwrap();
        let MoveOutcome::Queued(_) = mark else {
            panic!("mark committed while a gm seat was requested");
        };
        assert_eq!(shard.now().world_time, 1080);
        let outputs = shard.commit_then_fast_forward(&wall).unwrap();
        let reports = outputs.into_iter().next().unwrap().reports;
        let rowan = rolled_stat(&reports, 1);
        assert_eq!(shard.now().world_time, 1085);
        let tick = spookery_engine::testing::journal::read_record_line(&pack, "tick-0001")
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
    shard.seat_player("pc_rowan").unwrap();
    queue_gm(&mut shard, "gm-1", "pc_nell", "defend");
    let wall = shard.now().last_tick_at.clone();
    let queued =
        handle_player_move(&mut shard, "pc_rowan", "mark", Some("pc_nell"), true, &wall).unwrap();
    let MoveOutcome::Queued(_) = queued else {
        panic!("held mark committed before the round");
    };
    let before = shard.now().world_time;
    let outputs = shard.commit_then_fast_forward(&wall).unwrap();
    assert_ne!(shard.now().world_time, before);
    let rowan = rolled_stat(&outputs[0].reports, 1);
    assert_eq!(rowan.outcome, Outcome::Miss);
    assert!(
        !spookery_engine::testing::journal::read_record_line(&held, "tick-0001")
            .unwrap_or_else(|err| panic!("{}", err.detail))
            .contains("faces")
    );
    let _ = fs::remove_dir_all(&held);
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

#[when(expr = "prose is stripped")]
fn prose_is_stripped(_world: &mut World) {
    strip_bodies();
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
