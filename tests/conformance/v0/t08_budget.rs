use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use spookery_kernel::model::{
    ActorLive, Now, PauseView, PlaceLive, Presence, RunState, TickCounters,
};
use spookery_tick::tick_pack;

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

fn crowd(root: &Path, rules: &Path) {
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
    for index in 1..=50 {
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

#[test]
fn v0_t08_budget() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let pack = std::env::temp_dir().join(format!(
        "spook-tick08-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&pack);
    crowd(&pack, &manifest.join("packs/oneshot-docks/rules"));

    tick_pack(&pack).unwrap_or_else(|err| panic!("tick: {err}"));

    let shard =
        spookery_kernel::open_shard(&pack).unwrap_or_else(|err| panic!("open_shard: {err}"));
    let now = shard.now();
    assert_eq!(now.tick_index, 1);
    let npc_count = now.actors.values().filter(|actor| actor.is_npc()).count();
    assert_eq!(npc_count, 50);
    let acted = now
        .actors
        .values()
        .filter(|actor| actor.tick().is_some_and(|tick| tick.last_act_tick == 1))
        .count();
    let tape = fs::read_to_string(pack.join("state/log/tick-0001.jsonl"))
        .unwrap_or_else(|err| panic!("read tape: {err}"));
    let record: serde_json::Value =
        serde_json::from_str(tape.trim_end()).unwrap_or_else(|err| panic!("parse tape: {err}"));
    let applied = record
        .get("intents")
        .and_then(|intents| intents.as_array())
        .unwrap_or_else(|| panic!("intents missing"))
        .len();
    assert!(acted >= 1 && acted <= 8, "acted {acted}");
    assert!(applied >= 1 && applied <= 12, "applied {applied}");

    let _ = fs::remove_dir_all(&pack);
}
