use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use cucumber::{given, then, when};
use spookery_kernel::model::{
    ActorKind, ActorLive, Intent, IntentArgs, Now, PauseView, PlaceLive, Presence, RunState,
    Source, Target, TickCounters,
};

use crate::World;

const TRACE_A: &str = "\
{\"tick_index\":1,\"state_rev_before\":0,\"state_rev_after\":1,\"world_time_before\":1080,\"world_time_after\":1140,\"presence_humans\":0,\"seed\":\"docks-001|1\",\"last_tick_at\":\"2026-01-01T00:00:00Z\",\"intents\":[],\"rejected\":[],\"effects\":[],\"tool_trace\":[\"session_status\",\"tick_commit\"]}\n";

const TRACE_B: &str = "\
{\"tick_index\":1,\"state_rev_before\":0,\"state_rev_after\":1,\"world_time_before\":0,\"world_time_after\":5,\"presence_humans\":1,\"seed\":\"ex|1\",\"last_tick_at\":\"2026-01-01T00:00:00Z\",\"intents\":[{\"id\":\"00000000-0000-4000-8000-000000000001\",\"tick_index\":1,\"actor_id\":\"pc_rowan\",\"actor_kind\":\"player\",\"source\":\"player\",\"move\":\"wait\",\"target\":{},\"args\":{},\"rationale_code\":\"\",\"llm_say\":null}],\"rejected\":[],\"effects\":[],\"tool_trace\":[\"session_status\",\"perceive:pc_rowan\"]}\n";

const TRACE_C: &str = "\
{\"tick_index\":1,\"state_rev_before\":0,\"state_rev_after\":1,\"world_time_before\":0,\"world_time_after\":60,\"presence_humans\":0,\"seed\":\"cap|1\",\"last_tick_at\":\"2026-01-01T00:00:00Z\",\"intents\":[{\"id\":\"daa186f67613ae8ea1275b3fd03c38d52eea99e976235e499b5c2b0853c0d819\",\"tick_index\":1,\"actor_id\":\"n2\",\"actor_kind\":\"npc\",\"source\":\"kernel\",\"move\":\"work\",\"target\":{},\"args\":{},\"rationale_code\":\"\",\"llm_say\":null},{\"id\":\"0fe4c5c426ba070578c0f3b2db69a66a5b23fd1118b1403bb107c641c42156ee\",\"tick_index\":1,\"actor_id\":\"n3\",\"actor_kind\":\"npc\",\"source\":\"kernel\",\"move\":\"work\",\"target\":{},\"args\":{},\"rationale_code\":\"\",\"llm_say\":null},{\"id\":\"f8ffe70dfa42ab6fad611f54168246a7107403ac64f7f89298c6eae0b5253456\",\"tick_index\":1,\"actor_id\":\"n5\",\"actor_kind\":\"npc\",\"source\":\"kernel\",\"move\":\"work\",\"target\":{},\"args\":{},\"rationale_code\":\"\",\"llm_say\":null},{\"id\":\"6cdc1ede0d70121d3b48897b79fc3f91d2dcce0c00ed5f550e4963ae2e4430fa\",\"tick_index\":1,\"actor_id\":\"n4\",\"actor_kind\":\"npc\",\"source\":\"kernel\",\"move\":\"work\",\"target\":{},\"args\":{},\"rationale_code\":\"\",\"llm_say\":null}],\"rejected\":[{\"id\":\"7219eb49746a25b404200f3b6f070d3059ba154afd79f63dab2c847188b7130b\",\"reason\":\"budget\"}],\"effects\":[{\"class\":\"need\",\"as\":\"npc:n2\",\"intent_id\":\"daa186f67613ae8ea1275b3fd03c38d52eea99e976235e499b5c2b0853c0d819\",\"payload\":{\"actor_id\":\"n2\",\"key\":\"k1\",\"delta\":1}},{\"class\":\"need\",\"as\":\"npc:n2\",\"intent_id\":\"daa186f67613ae8ea1275b3fd03c38d52eea99e976235e499b5c2b0853c0d819\",\"payload\":{\"actor_id\":\"n2\",\"key\":\"k2\",\"delta\":1}},{\"class\":\"need\",\"as\":\"npc:n2\",\"intent_id\":\"daa186f67613ae8ea1275b3fd03c38d52eea99e976235e499b5c2b0853c0d819\",\"payload\":{\"actor_id\":\"n2\",\"key\":\"k3\",\"delta\":1}},{\"class\":\"need\",\"as\":\"npc:n2\",\"intent_id\":\"daa186f67613ae8ea1275b3fd03c38d52eea99e976235e499b5c2b0853c0d819\",\"payload\":{\"actor_id\":\"n2\",\"key\":\"k4\",\"delta\":1}},{\"class\":\"need\",\"as\":\"npc:n3\",\"intent_id\":\"0fe4c5c426ba070578c0f3b2db69a66a5b23fd1118b1403bb107c641c42156ee\",\"payload\":{\"actor_id\":\"n3\",\"key\":\"k1\",\"delta\":1}},{\"class\":\"need\",\"as\":\"npc:n3\",\"intent_id\":\"0fe4c5c426ba070578c0f3b2db69a66a5b23fd1118b1403bb107c641c42156ee\",\"payload\":{\"actor_id\":\"n3\",\"key\":\"k2\",\"delta\":1}},{\"class\":\"need\",\"as\":\"npc:n3\",\"intent_id\":\"0fe4c5c426ba070578c0f3b2db69a66a5b23fd1118b1403bb107c641c42156ee\",\"payload\":{\"actor_id\":\"n3\",\"key\":\"k3\",\"delta\":1}},{\"class\":\"need\",\"as\":\"npc:n3\",\"intent_id\":\"0fe4c5c426ba070578c0f3b2db69a66a5b23fd1118b1403bb107c641c42156ee\",\"payload\":{\"actor_id\":\"n3\",\"key\":\"k4\",\"delta\":1}},{\"class\":\"need\",\"as\":\"npc:n5\",\"intent_id\":\"f8ffe70dfa42ab6fad611f54168246a7107403ac64f7f89298c6eae0b5253456\",\"payload\":{\"actor_id\":\"n5\",\"key\":\"k1\",\"delta\":1}},{\"class\":\"need\",\"as\":\"npc:n5\",\"intent_id\":\"f8ffe70dfa42ab6fad611f54168246a7107403ac64f7f89298c6eae0b5253456\",\"payload\":{\"actor_id\":\"n5\",\"key\":\"k2\",\"delta\":1}},{\"class\":\"need\",\"as\":\"npc:n5\",\"intent_id\":\"f8ffe70dfa42ab6fad611f54168246a7107403ac64f7f89298c6eae0b5253456\",\"payload\":{\"actor_id\":\"n5\",\"key\":\"k3\",\"delta\":1}},{\"class\":\"need\",\"as\":\"npc:n5\",\"intent_id\":\"f8ffe70dfa42ab6fad611f54168246a7107403ac64f7f89298c6eae0b5253456\",\"payload\":{\"actor_id\":\"n5\",\"key\":\"k4\",\"delta\":1}},{\"class\":\"need\",\"as\":\"npc:n4\",\"intent_id\":\"6cdc1ede0d70121d3b48897b79fc3f91d2dcce0c00ed5f550e4963ae2e4430fa\",\"payload\":{\"actor_id\":\"n4\",\"key\":\"k1\",\"delta\":1}},{\"class\":\"need\",\"as\":\"npc:n4\",\"intent_id\":\"6cdc1ede0d70121d3b48897b79fc3f91d2dcce0c00ed5f550e4963ae2e4430fa\",\"payload\":{\"actor_id\":\"n4\",\"key\":\"k2\",\"delta\":1}},{\"class\":\"need\",\"as\":\"npc:n4\",\"intent_id\":\"6cdc1ede0d70121d3b48897b79fc3f91d2dcce0c00ed5f550e4963ae2e4430fa\",\"payload\":{\"actor_id\":\"n4\",\"key\":\"k3\",\"delta\":1}},{\"class\":\"need\",\"as\":\"npc:n4\",\"intent_id\":\"6cdc1ede0d70121d3b48897b79fc3f91d2dcce0c00ed5f550e4963ae2e4430fa\",\"payload\":{\"actor_id\":\"n4\",\"key\":\"k4\",\"delta\":1}}],\"tool_trace\":[\"session_status\",\"perceive:n2\",\"apply_effect\",\"apply_effect\",\"apply_effect\",\"apply_effect\",\"perceive:n3\",\"apply_effect\",\"apply_effect\",\"apply_effect\",\"apply_effect\",\"perceive:n5\",\"apply_effect\",\"apply_effect\",\"apply_effect\",\"apply_effect\",\"perceive:n4\",\"apply_effect\",\"apply_effect\",\"apply_effect\",\"apply_effect\",\"perceive:n1\",\"tick_commit\"]}\n";

fn fresh_pack(label: &str) -> PathBuf {
    let pack = std::env::temp_dir().join(format!(
        "spook-bdd-jsonl-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&pack);
    pack
}

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

fn copy_rules(root: &Path) {
    let rules = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("packs/oneshot-docks/rules");
    for name in ["moves.yaml", "checks.yaml", "tick.yaml"] {
        copy_file(&rules.join(name), &root.join("rules").join(name));
    }
}

fn rewrite_rev(root: &Path, now: Now) {
    let rev =
        spookery_kernel::canon::pack_rev(root).unwrap_or_else(|err| panic!("pack_rev: {err}"));
    let mut now = now;
    now.pack_rev = rev;
    let text =
        spookery_kernel::codec::emit_now(&now).unwrap_or_else(|err| panic!("emit_now: {err}"));
    write(&root.join("state/now.yaml"), &text);
}

fn npc(place: &str, needs: BTreeMap<String, i64>) -> ActorLive {
    ActorLive::Npc {
        place: place.to_string(),
        flags: Vec::new(),
        busy: false,
        needs,
        resources: BTreeMap::new(),
        relationships: BTreeMap::new(),
        reputation: BTreeMap::new(),
        known_facts: Vec::new(),
        tick: TickCounters {
            last_act_tick: 0,
            idle_acts_today: 0,
            live_acts_this_hour: 0,
        },
    }
}

fn pc(place: &str, sheet: &str) -> ActorLive {
    ActorLive::Pc {
        place: place.to_string(),
        flags: Vec::new(),
        busy: false,
        known_facts: Vec::new(),
        sheet: sheet.to_string(),
    }
}

fn base_now(shard_id: &str, world_time: u64, humans: u32, present: Vec<String>) -> Now {
    Now {
        schema: "now/v0".to_string(),
        shard_id: shard_id.to_string(),
        pack_rev: String::new(),
        state_rev: 0,
        tick_index: 0,
        world_time,
        last_tick_at: "2026-01-01T00:00:00Z".to_string(),
        presence: Presence {
            humans,
            session_ids: Vec::new(),
            actors_present: present,
        },
        pause: PauseView {
            named_plot_clocks: if humans == 0 {
                RunState::Paused
            } else {
                RunState::Running
            },
            relationship_drift: RunState::Running,
            subsistence_clocks: RunState::Running,
            travel: RunState::Running,
        },
        places: BTreeMap::new(),
        actors: BTreeMap::new(),
        clocks: BTreeMap::new(),
        rumors: Vec::new(),
        messages: Vec::new(),
        proposals: Vec::new(),
    }
}

fn place(actors: Vec<String>) -> PlaceLive {
    PlaceLive {
        stock: BTreeMap::new(),
        hours: BTreeMap::new(),
        notes: Vec::new(),
        actors,
    }
}

fn tick_line(pack: &Path, index: u64) -> String {
    fs::read_to_string(pack.join(format!("state/log/tick-{index:04}.jsonl")))
        .unwrap_or_else(|err| panic!("read tick {index}: {err}"))
}

fn assert_trace(w: &mut World, expected: &str) {
    let pack = w.pack.as_ref().unwrap();
    assert_eq!(tick_line(pack, 1), expected);
    let _ = fs::remove_dir_all(pack);
}

#[given(expr = "mira and bram are ineligible and the pack rev is rewritten")]
fn ineligible_and_rev(w: &mut World) {
    let pack = w.pack.clone().unwrap();
    for id in ["mira", "bram"] {
        let path = pack.join(format!("cast/{id}.md"));
        let text = fs::read_to_string(&path)
            .unwrap()
            .replace("eligible: true", "eligible: false");
        write(&path, &text);
    }
    let now_path = pack.join("state/now.yaml");
    let mut now =
        spookery_kernel::codec::load_now(&fs::read_to_string(&now_path).unwrap()).unwrap();
    now.pack_rev = spookery_kernel::canon::pack_rev(&pack).unwrap();
    write(&now_path, &spookery_kernel::codec::emit_now(&now).unwrap());
}

#[given(expr = "a gate pack with Rowan present at world_time 0")]
fn gate_pack(w: &mut World) {
    let pack = fresh_pack("b");
    write(
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
    write(
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
    write(
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
    write(
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
    copy_rules(&pack);
    let mut now = base_now("ex", 0, 1, vec!["pc_rowan".to_string()]);
    now.places
        .insert("gate".to_string(), place(vec!["pc_rowan".to_string()]));
    now.actors.insert(
        "pc_rowan".to_string(),
        pc("gate", "state/actors/pc_rowan.yaml"),
    );
    rewrite_rev(&pack, now);
    w.pack = Some(pack);
}

#[when(expr = "the queued player wait is committed")]
fn commit_wait(w: &mut World) {
    let pack = w.pack.clone().unwrap();
    let mut shard = spookery_kernel::open_shard(&pack).unwrap();
    shard.queue_intent(Intent {
        id: "00000000-0000-4000-8000-000000000001".to_string(),
        tick_index: 1,
        actor_id: "pc_rowan".to_string(),
        actor_kind: ActorKind::Player,
        source: Source::Player,
        move_name: "wait".to_string(),
        target: Target::Empty,
        args: IntentArgs::default(),
        rationale_code: String::new(),
    });
    spookery_tick::commit_turn(&mut shard).unwrap_or_else(|err| panic!("commit: {err}"));
}

#[given(expr = "a yard pack of five eligible NPCs with four work needs")]
fn yard_pack(w: &mut World) {
    let pack = fresh_pack("c");
    write(
        &pack.join("pack.yaml"),
        "\
schema: pack/v0
id: cap
title: Cap
system: cap
start:
  place: yard
  pc: n1
",
    );
    write(
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
    let mut needs = BTreeMap::new();
    for key in ["k1", "k2", "k3", "k4"] {
        needs.insert(key.to_string(), 0);
    }
    for id in ["n1", "n2", "n3", "n4", "n5"] {
        write(
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
  needs:
    k1:
      min: 0
      max: 5
    k2:
      min: 0
      max: 5
    k3:
      min: 0
      max: 5
    k4:
      min: 0
      max: 5
  work:
    needs:
      - key: k1
        delta: 1
      - key: k2
        delta: 1
      - key: k3
        delta: 1
      - key: k4
        delta: 1
    resources: []
---
{id}
"
            ),
        );
        actors.insert(id.to_string(), npc("yard", needs.clone()));
    }
    copy_rules(&pack);
    let mut now = base_now("cap", 0, 0, Vec::new());
    now.places.insert(
        "yard".to_string(),
        place(vec![
            "n1".to_string(),
            "n2".to_string(),
            "n3".to_string(),
            "n4".to_string(),
            "n5".to_string(),
        ]),
    );
    now.actors = actors;
    rewrite_rev(&pack, now);
    w.pack = Some(pack);
}

#[when(expr = "the kernel ticks this pack once")]
fn kernel_tick_once(w: &mut World) {
    let pack = w.pack.clone().unwrap();
    spookery_tick::tick_pack(&pack).unwrap_or_else(|err| panic!("tick: {err}"));
}

#[then(expr = "tick 1 jsonl equals the empty docks trace")]
fn trace_a(w: &mut World) {
    assert_trace(w, TRACE_A);
}

#[then(expr = "tick 1 jsonl equals the flushed wait trace")]
fn trace_b(w: &mut World) {
    assert_trace(w, TRACE_B);
}

#[then(expr = "tick 1 jsonl equals the five-NPC budget trace")]
fn trace_c(w: &mut World) {
    assert_trace(w, TRACE_C);
}
