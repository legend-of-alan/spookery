use std::fs;
use std::path::PathBuf;

use cucumber::{given, then, when};

use crate::support::spook;
use crate::World;

fn docks_rev() -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("packs/oneshot-docks");
    spookery_kernel::canon::pack_rev(&root).unwrap()
}

fn evidence(before_pack: &str, after_pack: &str, before_rev: u64, after_rev: u64) -> String {
    format!(
        "{{\"op\":\"migrate\",\"pack_rev_before\":\"{before_pack}\",\"pack_rev_after\":\"{after_pack}\",\"state_rev_before\":{before_rev},\"state_rev_after\":{after_rev}}}\n"
    )
}

fn open(w: &World) -> spookery_kernel::shard::Shard {
    spookery_kernel::open_shard(w.pack.as_ref().unwrap()).unwrap_or_else(|err| panic!("open: {err}"))
}

#[given(expr = "state\\/now.yaml is replaced with the schema failure document")]
fn schema_failure_document(w: &mut World) {
    let pack = w.pack.as_ref().unwrap();
    fs::write(pack.join("state/now.yaml"), "schema: no\n").unwrap();
}

#[when(expr = "now.yaml has the migrate hand edit")]
fn hand_edit(w: &mut World) {
    let pack = w.pack.as_ref().unwrap();
    let now_path = pack.join("state/now.yaml");
    let mut text = fs::read_to_string(&now_path).unwrap();
    text = text.replace("world_time: 1080", "world_time: 7");
    text = text.replace(
        "place: docks_shop\n    flags: []\n    busy: false\n    known_facts: []\n    sheet:",
        "place: mira_rooms\n    flags: []\n    busy: false\n    known_facts: []\n    sheet:",
    );
    text = text.replace(
        "    actors:\n      - aldric\n      - pc_rowan\n",
        "    actors:\n      - aldric\n",
    );
    text = text.replace(
        "    actors:\n      - mira\n",
        "    actors:\n      - mira\n      - pc_rowan\n",
    );
    fs::write(&now_path, text).unwrap();
}

#[when(expr = "spook migrate runs")]
fn migrate_runs(w: &mut World) {
    let pack = w.pack.as_ref().unwrap().clone();
    w.outputs
        .push(spook(&["migrate", pack.to_str().unwrap()]));
}

#[then(expr = "both migrate commands exit 0")]
fn both_exit_0(w: &mut World) {
    assert_eq!(w.outputs.len(), 2);
    for output in &w.outputs {
        assert!(
            output.status.success(),
            "stdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[then(expr = "the first migrate stdout and stderr are empty")]
fn first_silent(w: &mut World) {
    let first = &w.outputs[0];
    assert!(first.stdout.is_empty());
    assert!(first.stderr.is_empty());
}

#[then(expr = "the second migrate stdout is empty")]
fn second_stdout_empty(w: &mut World) {
    assert!(w.outputs[1].stdout.is_empty());
}

#[then(expr = "the migrate command exits 0")]
fn migrate_exits_0(w: &mut World) {
    let output = w.outputs.last().unwrap();
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[then(expr = "migrate exits 1 with stderr SCHEMA")]
fn migrate_schema(w: &mut World) {
    let output = w.outputs.last().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        (output.status.code(), stderr.trim()),
        (Some(1), "SCHEMA"),
        "stdout:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[then(expr = "now pack_rev is the docks pack rev")]
fn pack_rev_is_docks(w: &mut World) {
    assert_eq!(open(w).now().pack_rev, docks_rev());
}

#[then(expr = "the canon pack_rev differs from the docks pack rev")]
fn canon_differs(w: &mut World) {
    let rev = spookery_kernel::canon::pack_rev(w.pack.as_ref().unwrap()).unwrap();
    assert_ne!(rev, docks_rev());
}

#[then(expr = "now pack_rev equals the canon pack_rev")]
fn now_matches_canon(w: &mut World) {
    let pack = w.pack.as_ref().unwrap();
    let rev = spookery_kernel::canon::pack_rev(pack).unwrap();
    assert_eq!(open(w).now().pack_rev, rev);
}

#[then(expr = "actor {word} place is {word}")]
fn actor_place(w: &mut World, actor: String, place: String) {
    assert_eq!(open(w).now().actors.get(&actor).unwrap().place(), place);
}

#[then(expr = "state\\/log\\/tick-0001.jsonl does not exist")]
fn no_tick_log(w: &mut World) {
    assert!(!w
        .pack
        .as_ref()
        .unwrap()
        .join("state/log/tick-0001.jsonl")
        .exists());
}

#[then(expr = "state\\/log\\/studio.jsonl does not exist")]
fn no_studio(w: &mut World) {
    assert!(!w
        .pack
        .as_ref()
        .unwrap()
        .join("state/log/studio.jsonl")
        .exists());
}

#[then(expr = "state\\/log\\/studio.jsonl is the two migrate evidence lines")]
fn studio_two(w: &mut World) {
    let studio = fs::read_to_string(w.pack.as_ref().unwrap().join("state/log/studio.jsonl")).unwrap();
    assert_eq!(
        studio,
        format!(
            "{}{}",
            evidence(&docks_rev(), &docks_rev(), 0, 1),
            evidence(&docks_rev(), &docks_rev(), 1, 2)
        )
    );
}

#[then(expr = "state\\/log\\/studio.jsonl is the hand-edit migrate evidence")]
fn studio_hand(w: &mut World) {
    let pack = w.pack.as_ref().unwrap();
    let rev = spookery_kernel::canon::pack_rev(pack).unwrap();
    let studio = fs::read_to_string(pack.join("state/log/studio.jsonl")).unwrap();
    assert_eq!(studio, evidence(&docks_rev(), &rev, 0, 1));
}
