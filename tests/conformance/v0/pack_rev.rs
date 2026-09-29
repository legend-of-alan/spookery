use std::fs;
use std::path::PathBuf;
use std::process::Command;

use super::support::copy_dir;

#[test]
fn v0_pack_rev_mismatch_does_not_commit() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixture = manifest.join("packs/oneshot-docks");
    let pack = std::env::temp_dir().join(format!(
        "spook-packrev-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&pack);
    copy_dir(&fixture, &pack);

    let now_path = pack.join("state/now.yaml");
    let before = fs::read(&now_path).unwrap_or_else(|err| panic!("read now.yaml: {err}"));
    let mira = pack.join("cast/mira.md");
    let mut body = fs::read_to_string(&mira).unwrap_or_else(|err| panic!("read mira: {err}"));
    body.push('\n');
    fs::write(&mira, body).unwrap_or_else(|err| panic!("write mira: {err}"));

    let output = Command::new(env!("CARGO_BIN_EXE_spook"))
        .args(["tick", pack.to_str().unwrap()])
        .output()
        .unwrap_or_else(|err| panic!("spawn spook: {err}"));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        (output.status.code(), stderr.trim()),
        (Some(1), "PACK_REV_MISMATCH"),
        "stdout:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(
        fs::read(&now_path).unwrap_or_else(|err| panic!("reread now.yaml: {err}")),
        before,
        "tick committed after PACK_REV_MISMATCH"
    );
    assert!(
        !pack.join("state/.txn").exists(),
        "commit scratch was created"
    );
    assert!(!pack.join("state/log").exists(), "a tick file was written");
    match spookery_kernel::open_shard(&pack) {
        Err(spookery_kernel::Reason::PackRevMismatch) => {}
        Err(code) => panic!("expected PACK_REV_MISMATCH, got {code}"),
        Ok(_) => panic!("open_shard accepted a mismatched pack_rev"),
    }

    let _ = fs::remove_dir_all(&pack);
}
