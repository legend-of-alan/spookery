use std::process::Command;

#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;

use cucumber::{given, then, when};

use crate::World;

fn stock(shard: &spookery_kernel::shard::Shard) -> i64 {
    shard
        .now()
        .places
        .get("docks_shop")
        .and_then(|place| place.stock.get("lamp_oil"))
        .copied()
        .unwrap_or_else(|| panic!("lamp_oil missing"))
}

#[given(expr = "the shard state_rev, lamp_oil, and proposals are recorded")]
fn record_before(w: &mut World) {
    let shard = spookery_kernel::open_shard(w.pack.as_ref().unwrap())
        .unwrap_or_else(|err| panic!("open_shard: {err}"));
    w.scratch
        .insert("state_rev".to_string(), shard.now().state_rev.to_string());
    w.scratch
        .insert("lamp_oil".to_string(), stock(&shard).to_string());
    w.scratch.insert(
        "proposals".to_string(),
        format!("{:?}", shard.now().proposals),
    );
}

#[when(expr = "the pack is ticked with a crash after apply_effect")]
fn tick_crash_after_apply_effect(w: &mut World) {
    let pack = w.pack.as_ref().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_spook"))
        .args(["tick", pack.to_str().unwrap()])
        .env("SPOOK_CRASH_AFTER", "apply_effect")
        .output()
        .unwrap_or_else(|err| panic!("spawn spook: {err}"));
    w.outputs.push(output);
}

#[then(expr = "the tick is killed before commit")]
fn tick_killed(w: &mut World) {
    let output = w.outputs.last().unwrap();
    #[cfg(unix)]
    assert_eq!(
        output.status.signal(),
        Some(6),
        "kill after apply_effect\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    #[cfg(windows)]
    assert!(
        !output.status.success(),
        "kill after apply_effect\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[then(expr = "state\\/.txn does not exist")]
fn no_txn(w: &mut World) {
    assert!(
        !w.pack.as_ref().unwrap().join("state/.txn").exists(),
        "apply_effect created state/.txn"
    );
}

#[then(expr = "restart state_rev is unchanged")]
fn state_rev_unchanged(w: &mut World) {
    let shard = spookery_kernel::open_shard(w.pack.as_ref().unwrap())
        .unwrap_or_else(|err| panic!("restart: {err}"));
    let recorded: u64 = w.scratch.get("state_rev").unwrap().parse().unwrap();
    assert_eq!(shard.now().state_rev, recorded, "state_rev changed");
}

#[then(expr = "restart lamp_oil is unchanged")]
fn lamp_oil_unchanged(w: &mut World) {
    let shard = spookery_kernel::open_shard(w.pack.as_ref().unwrap())
        .unwrap_or_else(|err| panic!("restart: {err}"));
    let recorded: i64 = w.scratch.get("lamp_oil").unwrap().parse().unwrap();
    assert_eq!(stock(&shard), recorded, "stock value changed");
}

#[then(expr = "restart proposals are unchanged")]
fn proposals_unchanged(w: &mut World) {
    let shard = spookery_kernel::open_shard(w.pack.as_ref().unwrap())
        .unwrap_or_else(|err| panic!("restart: {err}"));
    let recorded = w.scratch.get("proposals").unwrap();
    assert_eq!(
        format!("{:?}", shard.now().proposals),
        *recorded,
        "proposal list changed"
    );
}

#[then(expr = "state\\/proposals does not exist")]
fn no_proposal_file(w: &mut World) {
    assert!(
        !w.pack.as_ref().unwrap().join("state/proposals").exists(),
        "a proposal file appeared"
    );
}

#[then(expr = "state\\/memories does not exist")]
fn no_memory_file(w: &mut World) {
    assert!(
        !w.pack.as_ref().unwrap().join("state/memories").exists(),
        "a memory file appeared"
    );
}
