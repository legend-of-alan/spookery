use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const LEDGER: &str = "The lamp-oil ledger is short.";

fn spook(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_spook"))
        .args(args)
        .output()
        .unwrap_or_else(|err| panic!("spawn spook: {err}"))
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap_or_else(|err| panic!("mkdir {}: {err}", to.display()));
    for entry in fs::read_dir(from).unwrap_or_else(|err| panic!("read {}: {err}", from.display())) {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        let file_type = entry.file_type().unwrap();
        if file_type.is_dir() {
            copy_dir(&entry.path(), &dest);
        } else {
            fs::copy(entry.path(), &dest)
                .unwrap_or_else(|err| panic!("copy {}: {err}", entry.path().display()));
        }
    }
}

fn read_optional(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

fn aldric_perceive(pack: &Path) -> String {
    // Firewall and perceive are not in this commit. Fail closed.
    // Do not filter facts in the test.
    let _ = pack;
    panic!("kernel::perceive is not landed");
}

#[test]
#[ignore = "empty fixture; firewall not landed"]
fn tick_01_closed_secret() {
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
    assert!(!aldric.contains(LEDGER), "fixture secret is absent on aldric");

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

    assert!(!perceive.contains(LEDGER), "ledger sentence in Aldric's perceive");
    assert!(!memories.contains(LEDGER), "ledger sentence in Aldric's memories");
    assert!(!rumors.contains(LEDGER), "ledger sentence in rumors");

    let _ = fs::remove_dir_all(&pack);
}
