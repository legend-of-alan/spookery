use std::fs;
use std::path::{Path, PathBuf};

use spookery_tick::{replay_recorded, tick_pack};

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
fn v0_t07_replay() {
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

    for tick in 1..=20 {
        tick_pack(&played).unwrap_or_else(|err| panic!("tick {tick}: {err}"));
    }
    let golden = fs::read(played.join("state/now.yaml")).unwrap_or_else(|err| panic!("read golden: {err}"));
    let source_log = fs::read(played.join("state/log/tick-0020.jsonl"))
        .unwrap_or_else(|err| panic!("read tape: {err}"));

    let temp = replay_recorded(&played, &start).unwrap_or_else(|err| panic!("replay: {err}"));
    let replayed = fs::read(temp.join("state/now.yaml")).unwrap_or_else(|err| panic!("read replay: {err}"));
    assert_eq!(replayed, golden);
    assert_ne!(temp, played);
    assert_eq!(
        fs::read(played.join("state/now.yaml")).unwrap(),
        golden,
        "replay wrote the source pack"
    );
    assert_eq!(
        fs::read(played.join("state/log/tick-0020.jsonl")).unwrap(),
        source_log,
        "replay wrote the source tape"
    );

    let _ = fs::remove_dir_all(&start);
    let _ = fs::remove_dir_all(&played);
    let _ = fs::remove_dir_all(&temp);
}
