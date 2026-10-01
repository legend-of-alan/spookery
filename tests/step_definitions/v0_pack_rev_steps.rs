use std::fs;

use cucumber::{given, then, when};

use crate::support::spook;
use crate::World;

#[given(expr = "state\\/now.yaml bytes are recorded")]
fn record_now(w: &mut World) {
    let pack = w.pack.as_ref().unwrap();
    let before = fs::read(pack.join("state/now.yaml"))
        .unwrap_or_else(|err| panic!("read now.yaml: {err}"));
    let before = String::from_utf8(before).unwrap_or_else(|err| panic!("now.yaml utf-8: {err}"));
    w.scratch.insert("now.yaml".to_string(), before);
}

#[when(expr = "a newline is appended to cast\\/mira.md")]
fn append_mira_newline(w: &mut World) {
    let pack = w.pack.as_ref().unwrap();
    let mira = pack.join("cast/mira.md");
    let mut body = fs::read_to_string(&mira).unwrap_or_else(|err| panic!("read mira: {err}"));
    body.push('\n');
    fs::write(&mira, body).unwrap_or_else(|err| panic!("write mira: {err}"));
}

#[when(expr = "the copied pack is ticked once")]
fn tick_once(w: &mut World) {
    let pack = w.pack.as_ref().unwrap();
    w.outputs
        .push(spook(&["tick", pack.to_str().unwrap()]));
}

#[then(expr = "the tick exits {int} with stderr {word}")]
fn tick_exits(w: &mut World, code: i64, expected: String) {
    let output = w.outputs.last().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        (output.status.code(), stderr.trim()),
        (Some(code as i32), expected.as_str()),
        "stdout:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[then(expr = "state\\/now.yaml bytes are unchanged")]
fn now_unchanged(w: &mut World) {
    let pack = w.pack.as_ref().unwrap();
    let before = w.scratch.get("now.yaml").unwrap();
    let after = fs::read(pack.join("state/now.yaml"))
        .unwrap_or_else(|err| panic!("reread now.yaml: {err}"));
    assert_eq!(
        after,
        before.as_bytes(),
        "tick committed after PACK_REV_MISMATCH"
    );
}

#[then(expr = "no commit scratch was created")]
fn no_commit_scratch(w: &mut World) {
    assert!(
        !w.pack.as_ref().unwrap().join("state/.txn").exists(),
        "commit scratch was created"
    );
}

#[then(expr = "state\\/log does not exist")]
fn no_tick_log(w: &mut World) {
    assert!(
        !w.pack.as_ref().unwrap().join("state/log").exists(),
        "a tick file was written"
    );
}

#[then(expr = "open_shard returns PACK_REV_MISMATCH")]
fn open_shard_mismatch(w: &mut World) {
    match spookery_kernel::open_shard(w.pack.as_ref().unwrap()) {
        Err(spookery_kernel::Reason::PackRevMismatch) => {}
        Err(code) => panic!("expected PACK_REV_MISMATCH, got {code}"),
        Ok(_) => panic!("open_shard accepted a mismatched pack_rev"),
    }
}
