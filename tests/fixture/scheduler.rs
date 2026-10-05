use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use spookery_kernel::model::{ActorLive, Now, PauseView, PlaceLive, Presence, RunState};

fn write(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .unwrap_or_else(|err| panic!("mkdir {}: {err}", parent.display()));
    }
    fs::write(path, text).unwrap_or_else(|err| panic!("write {}: {err}", path.display()));
}

fn pack() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "spook-scheduler-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&root);
    write(
        &root.join("pack.yaml"),
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
    write(
        &root.join("world/places/gate.md"),
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
    write(
        &root.join("cast/pc_rowan.md"),
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
    write(
        &root.join("state/actors/pc_rowan.yaml"),
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
    let rules = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("packs/oneshot-docks/rules");
    for name in ["moves.yaml", "checks.yaml"] {
        let text = fs::read_to_string(rules.join(name)).unwrap();
        write(&root.join("rules").join(name), &text);
    }
    let tick = fs::read_to_string(rules.join("tick.yaml")).unwrap();
    write(
        &root.join("rules/tick.yaml"),
        &tick.replace("tick_every_real_seconds: 300", "tick_every_real_seconds: 1"),
    );
    let rev = spookery_kernel::canon::pack_rev(&root).unwrap();
    let mut places = std::collections::BTreeMap::new();
    places.insert(
        "gate".to_string(),
        PlaceLive {
            stock: std::collections::BTreeMap::new(),
            hours: std::collections::BTreeMap::new(),
            notes: Vec::new(),
            actors: vec!["pc_rowan".to_string()],
        },
    );
    let mut actors = std::collections::BTreeMap::new();
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
        clocks: std::collections::BTreeMap::new(),
        rumors: Vec::new(),
        messages: Vec::new(),
        proposals: Vec::new(),
    };
    let text = spookery_kernel::codec::emit_now(&now).unwrap();
    write(&root.join("state/now.yaml"), &text);
    root
}

pub(crate) async fn scheduler_sleeps_then_catches_up() {
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
    let root = pack();
    let started = Instant::now();
    spookery::schedule::run_wake(&root)
        .await
        .unwrap_or_else(|err| panic!("wake: {err}"));
    let elapsed = started.elapsed();
    assert!(
        elapsed >= Duration::from_secs(1),
        "slept {:?}, shorter than tick_every_real_seconds",
        elapsed
    );
    assert!(elapsed < Duration::from_secs(20), "slept {elapsed:?}");
    let shard = spookery_kernel::open_shard(&root).unwrap();
    assert_eq!(shard.now().tick_index, 12);
    assert_eq!(shard.now().state_rev, 12);
    assert_eq!(shard.now().world_time, 720);
    assert_eq!(shard.now().proposals, vec!["season_advance".to_string()]);
    assert_ne!(shard.now().last_tick_at, "2026-01-01T00:00:00Z");
    assert_eq!(shard.now().last_tick_at.len(), 20);
    assert!(shard.now().last_tick_at.ends_with('Z'));
    let _ = fs::remove_dir_all(&root);
}
