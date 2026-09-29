use std::fs;
use std::path::{Path, PathBuf};

use super::support::{copy_dir, spook};

const LEDGER: &str = "The lamp-oil ledger is short.";

fn read_optional(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

fn aldric_perceive(pack: &Path) -> String {
    let shard = spookery_kernel::open_shard(pack).unwrap_or_else(|err| panic!("open_shard: {err}"));
    let view = spookery_kernel::perceive(&shard, "npc:aldric")
        .unwrap_or_else(|err| panic!("perceive: {err}"));
    format!("{view:?}")
}

#[test]
fn v0_t01_closed_secret() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixture = manifest.join("packs/oneshot-docks");
    let pack = std::env::temp_dir().join(format!(
        "spook-tick01-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&pack);
    copy_dir(&fixture, &pack);

    let mira = fs::read_to_string(pack.join("cast/mira.md")).unwrap();
    let aldric = fs::read_to_string(pack.join("cast/aldric.md")).unwrap();
    assert!(mira.contains(LEDGER), "fixture secret is on mira");
    assert!(
        !aldric.contains(LEDGER),
        "fixture secret is absent on aldric"
    );

    for tick in 1..=100 {
        let output = spook(&["tick", pack.to_str().unwrap()]);
        assert!(
            output.status.success(),
            "idle tick {tick}\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let perceive = aldric_perceive(&pack);
    let memories = read_optional(&pack.join("state/memories/aldric.jsonl"));
    let rumors = format!(
        "{}\n{}",
        read_optional(&pack.join("state/rumors.yaml")),
        read_optional(&pack.join("state/now.yaml"))
    );

    assert!(
        !perceive.contains(LEDGER),
        "ledger sentence in Aldric's perceive"
    );
    assert!(
        !memories.contains(LEDGER),
        "ledger sentence in Aldric's memories"
    );
    assert!(!rumors.contains(LEDGER), "ledger sentence in rumors");

    let _ = fs::remove_dir_all(&pack);
}
