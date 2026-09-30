use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use cucumber::{given, then, when};
use spookery_kernel::model::{ActorLive, Now, PauseView, PlaceLive, Presence, RunState};
use spookery_tick::run_catchup;

use crate::World;

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

const TRACE_D_12: &str = "\
{\"tick_index\":12,\"state_rev_before\":11,\"state_rev_after\":12,\"world_time_before\":660,\"world_time_after\":720,\"presence_humans\":0,\"seed\":\"ex|12\",\"last_tick_at\":\"2026-01-01T00:00:00Z\",\"intents\":[{\"id\":\"793b67bc1f8a5ded18c1375e2c371758b0c9063500e69195df66ad001fff669a\",\"tick_index\":12,\"actor_id\":\"pc_rowan\",\"actor_kind\":\"player\",\"source\":\"system\",\"move\":\"file_proposal\",\"target\":{},\"args\":{\"proposal_id\":\"season_advance\",\"kind\":\"other\",\"target\":\"\",\"summary\":\"season_advance\"},\"rationale_code\":\"\",\"llm_say\":null}],\"rejected\":[],\"effects\":[{\"class\":\"proposal\",\"as\":\"player:pc_rowan\",\"intent_id\":\"793b67bc1f8a5ded18c1375e2c371758b0c9063500e69195df66ad001fff669a\",\"payload\":{\"proposal_id\":\"season_advance\",\"kind\":\"other\",\"target\":\"\",\"summary\":\"season_advance\"}}],\"tool_trace\":[\"session_status\",\"apply_effect\",\"tick_commit\"]}\n";

fn copy_file(from: &Path, to: &Path) {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)
            .unwrap_or_else(|err| panic!("mkdir {}: {err}", parent.display()));
    }
    fs::copy(from, to).unwrap_or_else(|err| panic!("copy {}: {err}", from.display()));
}

fn write(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .unwrap_or_else(|err| panic!("mkdir {}: {err}", parent.display()));
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
    let rev =
        spookery_kernel::canon::pack_rev(root).unwrap_or_else(|err| panic!("pack_rev: {err}"));
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
    let text =
        spookery_kernel::codec::emit_now(&now).unwrap_or_else(|err| panic!("emit_now: {err}"));
    write(&root.join("state/now.yaml"), &text);
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut names = Vec::new();
    if dir.is_dir() {
        for entry in fs::read_dir(dir).unwrap_or_else(|err| panic!("read {}: {err}", dir.display()))
        {
            let entry = entry.unwrap_or_else(|err| panic!("dirent: {err}"));
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    names.sort();
    names
}

fn empty_idle(n: u64, shard: &str) -> String {
    format!(
        "{{\"tick_index\":{n},\"state_rev_before\":{},\"state_rev_after\":{n},\"world_time_before\":{},\"world_time_after\":{},\"presence_humans\":0,\"seed\":\"{shard}|{n}\",\"last_tick_at\":\"2026-01-01T00:00:00Z\",\"intents\":[],\"rejected\":[],\"effects\":[],\"tool_trace\":[\"session_status\",\"tick_commit\"]}}\n",
        n - 1,
        (n - 1) * 60,
        n * 60,
    )
}

fn pack_of(w: &World) -> &Path {
    w.pack.as_ref().unwrap()
}

#[given(expr = "shard ex from world_time 0")]
fn shard_ex(w: &mut World) {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let pack = std::env::temp_dir().join(format!(
        "spook-bdd-t04-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&pack);
    trace_pack(&pack, &manifest.join("packs/oneshot-docks/rules"));
    w.scratch.insert(
        "gate.md".to_string(),
        fs::read_to_string(pack.join("world/places/gate.md"))
            .unwrap_or_else(|err| panic!("read gate: {err}")),
    );
    w.scratch.insert(
        "pc_rowan.md".to_string(),
        fs::read_to_string(pack.join("cast/pc_rowan.md"))
            .unwrap_or_else(|err| panic!("read cast: {err}")),
    );
    let shard = spookery_kernel::open_shard(&pack).unwrap_or_else(|err| panic!("open_shard: {err}"));
    assert_eq!(shard.now().shard_id, "ex");
    assert_eq!(shard.now().world_time, 0);
    w.pack = Some(pack);
}

#[when(expr = "catch-up runs for {int} hours of downtime")]
fn catchup_hours(w: &mut World, hours: u64) {
    let mut shard =
        spookery_kernel::open_shard(pack_of(w)).unwrap_or_else(|err| panic!("open_shard: {err}"));
    let wall = shard.now().last_tick_at.clone();
    run_catchup(&mut shard, &wall, hours).unwrap_or_else(|err| panic!("catch-up: {err}"));
}

#[then(expr = "state_rev is {int}")]
fn state_rev_is(w: &mut World, n: u64) {
    let shard =
        spookery_kernel::open_shard(pack_of(w)).unwrap_or_else(|err| panic!("reopen: {err}"));
    assert_eq!(shard.now().state_rev, n, "twelve commits");
}

#[then(expr = "world_time is {int}")]
fn world_time_is(w: &mut World, n: u64) {
    let shard =
        spookery_kernel::open_shard(pack_of(w)).unwrap_or_else(|err| panic!("reopen: {err}"));
    assert_eq!(shard.now().world_time, n);
}

#[then(expr = "last_tick_at is 2026-01-01T00:00:00Z")]
fn last_tick_at_is(w: &mut World) {
    let shard =
        spookery_kernel::open_shard(pack_of(w)).unwrap_or_else(|err| panic!("reopen: {err}"));
    assert_eq!(shard.now().last_tick_at, "2026-01-01T00:00:00Z");
}

#[then(expr = "proposals are season_advance")]
fn proposals_are(w: &mut World) {
    let shard =
        spookery_kernel::open_shard(pack_of(w)).unwrap_or_else(|err| panic!("reopen: {err}"));
    assert_eq!(shard.now().proposals, vec!["season_advance".to_string()]);
}

#[then(expr = "state\\/log holds {int} tick jsonl files")]
fn log_holds(w: &mut World, n: usize) {
    let count = names_in(&pack_of(w).join("state/log"))
        .into_iter()
        .filter(|name| name.starts_with("tick-") && name.ends_with(".jsonl"))
        .count();
    assert_eq!(count, n);
}

#[then(expr = "state\\/proposals holds season_advance.yaml")]
fn proposals_dir(w: &mut World) {
    assert_eq!(
        names_in(&pack_of(w).join("state/proposals")),
        vec!["season_advance.yaml".to_string()]
    );
}

#[then(expr = "proposal season_advance.yaml equals the recorded season_advance document")]
fn proposal_document(w: &mut World) {
    let proposal = fs::read_to_string(pack_of(w).join("state/proposals/season_advance.yaml"))
        .unwrap_or_else(|err| panic!("read proposal: {err}"));
    assert_eq!(proposal, PROPOSAL);
}

#[then(expr = "world\\/places\\/gate.md is unchanged")]
fn gate_unchanged(w: &mut World) {
    let gate = fs::read_to_string(pack_of(w).join("world/places/gate.md"))
        .unwrap_or_else(|err| panic!("read gate: {err}"));
    assert_eq!(gate, w.scratch["gate.md"]);
}

#[then(expr = "cast\\/pc_rowan.md is unchanged")]
fn cast_unchanged(w: &mut World) {
    let cast = fs::read_to_string(pack_of(w).join("cast/pc_rowan.md"))
        .unwrap_or_else(|err| panic!("read cast: {err}"));
    assert_eq!(cast, w.scratch["pc_rowan.md"]);
}

#[then(expr = "tick jsonl lines 1 through 11 are empty idle lines for shard ex")]
fn idle_lines(w: &mut World) {
    let pack = pack_of(w);
    for n in 1..=11 {
        let line = fs::read_to_string(pack.join(format!("state/log/tick-{n:04}.jsonl")))
            .unwrap_or_else(|err| panic!("read tick {n}: {err}"));
        assert_eq!(line, empty_idle(n, "ex"), "tick {n}");
    }
}

#[then(expr = "tick jsonl 12 is the season_advance catch-up line")]
fn tick_12_line(w: &mut World) {
    let line = fs::read_to_string(pack_of(w).join("state/log/tick-0012.jsonl"))
        .unwrap_or_else(|err| panic!("read tick 12: {err}"));
    assert_eq!(line, TRACE_D_12);
}
