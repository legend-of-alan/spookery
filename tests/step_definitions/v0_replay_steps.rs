use std::fs;
use std::path::PathBuf;

use cucumber::{given, then, when};

use crate::support::copy_dir;
use crate::World;

fn saved_path(w: &World, key: &str) -> PathBuf {
    PathBuf::from(
        w.scratch
            .get(key)
            .unwrap_or_else(|| panic!("missing {key}")),
    )
}

fn read_utf8(path: &std::path::Path) -> String {
    let bytes = fs::read(path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
    String::from_utf8(bytes).unwrap_or_else(|err| panic!("utf-8 {}: {err}", path.display()))
}

#[given(expr = "a temp start copy and a played copy of pack packs\\/oneshot-docks")]
fn copy_start_and_played(w: &mut World) {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let stamp = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let start = std::env::temp_dir().join(format!("spook-tick07-start-{stamp}"));
    let played = std::env::temp_dir().join(format!("spook-tick07-played-{stamp}"));
    let _ = fs::remove_dir_all(&start);
    let _ = fs::remove_dir_all(&played);
    copy_dir(&manifest.join("packs/oneshot-docks"), &start);
    copy_dir(&start, &played);
    w.scratch
        .insert("start".to_string(), start.to_str().unwrap().to_string());
    w.scratch
        .insert("played".to_string(), played.to_str().unwrap().to_string());
}

#[when(expr = "the played pack is kernel-ticked {int} times and replayed onto the start snapshot")]
fn kernel_tick_and_replay(w: &mut World, n: usize) {
    let played = saved_path(w, "played");
    let start = saved_path(w, "start");
    for tick in 1..=n {
        spookery_tick::tick_pack(&played).unwrap_or_else(|err| panic!("tick {tick}: {err}"));
    }
    let golden = read_utf8(&played.join("state/now.yaml"));
    let source_log = read_utf8(&played.join(format!("state/log/tick-{n:04}.jsonl")));
    let temp = spookery_tick::replay_recorded(&played, &start)
        .unwrap_or_else(|err| panic!("replay: {err}"));
    w.scratch.insert("golden".to_string(), golden);
    w.scratch.insert("source_log".to_string(), source_log);
    w.scratch
        .insert("replay".to_string(), temp.to_str().unwrap().to_string());
}

#[then(expr = "the replay now.yaml bytes equal the golden now.yaml")]
fn replay_now_matches(w: &mut World) {
    let replay = saved_path(w, "replay");
    let golden = w
        .scratch
        .get("golden")
        .unwrap_or_else(|| panic!("missing golden"));
    let replayed =
        fs::read(replay.join("state/now.yaml")).unwrap_or_else(|err| panic!("read replay: {err}"));
    assert_eq!(replayed, golden.as_bytes());
}

#[then(expr = "the replay directory is not the played pack")]
fn replay_dir_is_distinct(w: &mut World) {
    assert_ne!(saved_path(w, "replay"), saved_path(w, "played"));
}

#[then(expr = "the played now.yaml bytes still equal the golden now.yaml")]
fn source_now_unchanged(w: &mut World) {
    let played = saved_path(w, "played");
    let golden = w
        .scratch
        .get("golden")
        .unwrap_or_else(|| panic!("missing golden"));
    let now =
        fs::read(played.join("state/now.yaml")).unwrap_or_else(|err| panic!("read golden: {err}"));
    assert_eq!(now, golden.as_bytes(), "replay wrote the source pack");
}

#[then(expr = "the played tick log tick-0020.jsonl bytes are unchanged")]
fn source_tape_unchanged(w: &mut World) {
    let played = saved_path(w, "played");
    let source_log = w
        .scratch
        .get("source_log")
        .unwrap_or_else(|| panic!("missing source_log"));
    let tape = fs::read(played.join("state/log/tick-0020.jsonl"))
        .unwrap_or_else(|err| panic!("read tape: {err}"));
    assert_eq!(tape, source_log.as_bytes(), "replay wrote the source tape");
    for key in ["start", "played", "replay"] {
        let _ = fs::remove_dir_all(saved_path(w, key));
    }
}
