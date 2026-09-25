use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use spookery_kernel::model::{ActorLive, Now, PauseView, PlaceLive, Presence, RunState};
use spookery_tick::run_catchup;

const PROPOSAL: &str = "\
schema: proposal/v0
id: season_advance
kind: other
target: \"\"
summary: season_advance
actor_id: pc_rowan
source: system
tick_index: 12
";

fn copy_file(from: &Path, to: &Path) {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).unwrap_or_else(|err| panic!("mkdir {}: {err}", parent.display()));
    }
    fs::copy(from, to).unwrap_or_else(|err| panic!("copy {}: {err}", from.display()));
}

fn write(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap_or_else(|err| panic!("mkdir {}: {err}", parent.display()));
    }
    fs::write(path, text).unwrap_or_else(|err| panic!("write {}: {err}", path.display()));
}

fn trace_pack(root: &Path, rules: &Path) {
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
    for name in ["moves.yaml", "checks.yaml", "tick.yaml"] {
        copy_file(&rules.join(name), &root.join("rules").join(name));
    }
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
    let rev = spookery_kernel::canon::pack_rev(root).unwrap_or_else(|err| panic!("pack_rev: {err}"));
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
    let text = spookery_kernel::codec::emit_now(&now).unwrap_or_else(|err| panic!("emit_now: {err}"));
    write(&root.join("state/now.yaml"), &text);
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut names = Vec::new();
    if dir.is_dir() {
        for entry in fs::read_dir(dir).unwrap_or_else(|err| panic!("read {}: {err}", dir.display())) {
            let entry = entry.unwrap_or_else(|err| panic!("dirent: {err}"));
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    names.sort();
    names
}

#[test]
fn tick_04_catchup_cap() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let pack = std::env::temp_dir().join(format!(
        "spook-tick04-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&pack);
    trace_pack(&pack, &manifest.join("packs/oneshot-docks/rules"));
    let place_before = fs::read(pack.join("world/places/gate.md")).unwrap();
    let cast_before = fs::read(pack.join("cast/pc_rowan.md")).unwrap();

    let mut shard = spookery_kernel::open_shard(&pack).unwrap_or_else(|err| panic!("open_shard: {err}"));
    let wall = shard.now().last_tick_at.clone();
    run_catchup(&mut shard, &wall, 72).unwrap_or_else(|err| panic!("catch-up: {err}"));

    let saved = spookery_kernel::open_shard(&pack).unwrap_or_else(|err| panic!("reopen: {err}"));
    let now = saved.now();
    assert_eq!(now.state_rev, 12, "twelve commits");
    assert_eq!(now.tick_index, 12);
    assert_eq!(now.world_time, 720);
    assert_eq!(now.last_tick_at, "2026-01-01T00:00:00Z");
    assert_eq!(now.proposals, vec!["season_advance".to_string()]);
    assert_eq!(
        names_in(&pack.join("state/log"))
            .into_iter()
            .filter(|name| name.starts_with("tick-") && name.ends_with(".jsonl"))
            .count(),
        12
    );
    assert_eq!(names_in(&pack.join("state/proposals")), vec!["season_advance.yaml".to_string()]);
    let proposal = fs::read_to_string(pack.join("state/proposals/season_advance.yaml"))
        .unwrap_or_else(|err| panic!("read proposal: {err}"));
    assert_eq!(proposal, PROPOSAL);
    assert_eq!(fs::read(pack.join("world/places/gate.md")).unwrap(), place_before);
    assert_eq!(fs::read(pack.join("cast/pc_rowan.md")).unwrap(), cast_before);

    let _ = fs::remove_dir_all(&pack);
}
