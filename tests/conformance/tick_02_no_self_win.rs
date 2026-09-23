use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

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

#[test]
fn tick_02_no_self_win() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixture = manifest.join("packs/oneshot-docks");
    let pack = std::env::temp_dir().join(format!(
        "spook-tick02-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&pack);
    copy_dir(&fixture, &pack);

    for tick in 1..=12 {
        let output = spook(&["tick", pack.to_str().unwrap()]);
        assert!(
            output.status.success(),
            "tick {tick}\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let shard = spookery_kernel::open_shard(&pack).unwrap_or_else(|err| panic!("open_shard: {err}"));
    let now = shard.now();
    assert_eq!(now.tick_index, 12, "twelve ticks commit");
    let clock = now
        .clocks
        .get("assassination_debt")
        .unwrap_or_else(|| panic!("assassination_debt missing"));
    assert_eq!(clock.value, 2);
    assert_eq!(clock.max, 6);
    assert!(clock.paused);
    assert!(clock.requires_presence);
    assert_eq!(clock.owners, vec!["pc_rowan".to_string()]);

    let _ = fs::remove_dir_all(&pack);
}
