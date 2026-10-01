use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use cucumber::{given, then, when};
use spookery_kernel::model::{
    ActorLive, Now, PauseView, PlaceLive, Presence, RunState, TickCounters,
};

use crate::World;

fn write(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .unwrap_or_else(|err| panic!("mkdir {}: {err}", parent.display()));
    }
    fs::write(path, text).unwrap_or_else(|err| panic!("write {}: {err}", path.display()));
}

fn copy_file(from: &Path, to: &Path) {
    write(
        to,
        &fs::read_to_string(from).unwrap_or_else(|err| panic!("read {}: {err}", from.display())),
    );
}

fn crowd(root: &Path, rules: &Path, count: i64) {
    write(
        &root.join("pack.yaml"),
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
    write(
        &root.join("world/places/yard.md"),
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
    for index in 1..=count {
        let id = format!("n{index}");
        write(
            &root.join(format!("cast/{id}.md")),
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
        copy_file(&rules.join(name), &root.join("rules").join(name));
    }
    let rev =
        spookery_kernel::canon::pack_rev(root).unwrap_or_else(|err| panic!("pack_rev: {err}"));
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
            session_ids: Vec::new(),
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
    let text =
        spookery_kernel::codec::emit_now(&now).unwrap_or_else(|err| panic!("emit_now: {err}"));
    write(&root.join("state/now.yaml"), &text);
}

#[given(expr = "a crowd pack of {int} eligible NPCs using packs\\/oneshot-docks rules")]
fn crowd_pack(w: &mut World, count: i64) {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let pack = std::env::temp_dir().join(format!(
        "spook-bdd-t08-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&pack);
    crowd(
        &pack,
        &manifest.join("packs/oneshot-docks/rules"),
        count,
    );
    w.pack = Some(pack);
}

#[when(expr = "the crowd pack is ticked once")]
fn tick_crowd(w: &mut World) {
    let pack = w.pack.as_ref().unwrap();
    spookery_tick::tick_pack(pack).unwrap_or_else(|err| panic!("tick: {err}"));
}

#[then(expr = "the shard has {int} NPCs")]
fn npc_count_is(w: &mut World, n: i64) {
    let shard = spookery_kernel::open_shard(w.pack.as_ref().unwrap()).unwrap();
    let npc_count = shard
        .now()
        .actors
        .values()
        .filter(|actor| actor.is_npc())
        .count();
    assert_eq!(npc_count as i64, n);
}

#[then(expr = "between {int} and {int} NPCs acted on tick {int}")]
fn acted_between(w: &mut World, lo: i64, hi: i64, tick: i64) {
    let shard = spookery_kernel::open_shard(w.pack.as_ref().unwrap()).unwrap();
    let acted = shard
        .now()
        .actors
        .values()
        .filter(|actor| {
            actor
                .tick()
                .is_some_and(|counters| counters.last_act_tick == tick as u64)
        })
        .count();
    assert!(
        acted as i64 >= lo && acted as i64 <= hi,
        "acted {acted}"
    );
}

#[then(
    expr = "state\\/log\\/tick-0001.jsonl records between {int} and {int} applied intents"
)]
fn applied_between(w: &mut World, lo: i64, hi: i64) {
    let pack = w.pack.as_ref().unwrap();
    let tape = fs::read_to_string(pack.join("state/log/tick-0001.jsonl"))
        .unwrap_or_else(|err| panic!("read tape: {err}"));
    let record: serde_json::Value =
        serde_json::from_str(tape.trim_end()).unwrap_or_else(|err| panic!("parse tape: {err}"));
    let applied = record
        .get("intents")
        .and_then(|intents| intents.as_array())
        .unwrap_or_else(|| panic!("intents missing"))
        .len();
    assert!(
        applied as i64 >= lo && applied as i64 <= hi,
        "applied {applied}"
    );
    let _ = fs::remove_dir_all(pack);
}
