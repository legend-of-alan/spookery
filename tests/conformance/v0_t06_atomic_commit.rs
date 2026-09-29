use std::fs;
#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;
use std::process::Command;

use super::support::copy_dir;

fn stock(shard: &spookery_kernel::store::Shard) -> i64 {
    shard
        .now()
        .places
        .get("docks_shop")
        .and_then(|place| place.stock.get("lamp_oil"))
        .copied()
        .unwrap_or_else(|| panic!("lamp_oil missing"))
}

#[test]
fn v0_t06_atomic_commit() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixture = manifest.join("packs/oneshot-docks");
    let pack = std::env::temp_dir().join(format!(
        "spook-tick06-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&pack);
    copy_dir(&fixture, &pack);

    let before =
        spookery_kernel::open_shard(&pack).unwrap_or_else(|err| panic!("open_shard: {err}"));
    let state_rev = before.now().state_rev;
    let lamp_oil = stock(&before);
    let proposals = before.now().proposals.clone();
    drop(before);

    let output = Command::new(env!("CARGO_BIN_EXE_spook"))
        .args(["tick", pack.to_str().unwrap()])
        .env("SPOOK_CRASH_AFTER", "apply_effect")
        .output()
        .unwrap_or_else(|err| panic!("spawn spook: {err}"));
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

    assert!(
        !pack.join("state/.txn").exists(),
        "apply_effect created state/.txn"
    );

    let restarted =
        spookery_kernel::open_shard(&pack).unwrap_or_else(|err| panic!("restart: {err}"));
    assert_eq!(restarted.now().state_rev, state_rev, "state_rev changed");
    assert_eq!(stock(&restarted), lamp_oil, "stock value changed");
    assert_eq!(
        restarted.now().proposals,
        proposals,
        "proposal list changed"
    );
    assert!(
        !pack.join("state/proposals").exists(),
        "a proposal file appeared"
    );
    assert!(
        !pack.join("state/memories").exists(),
        "a memory file appeared"
    );

    let _ = fs::remove_dir_all(&pack);
}
