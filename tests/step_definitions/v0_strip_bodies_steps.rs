use std::fs;
use std::path::{Path, PathBuf};

use cucumber::{given, then, when};

use crate::support::copy_dir;
use crate::World;

const LEDGER: &str = "The lamp-oil ledger is short.";

fn markdown_files(dir: &Path, out: &mut Vec<PathBuf>) {
    if !dir.exists() {
        return;
    }
    let mut entries = fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("read {}: {err}", dir.display()))
        .collect::<Result<Vec<_>, _>>()
        .unwrap_or_else(|err| panic!("dirent {}: {err}", dir.display()));
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            markdown_files(&path, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("md") {
            out.push(path);
        }
    }
}

fn unique_dir(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "spook-bdd-c11-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ))
}

fn strip_pack(label: &str) -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let pack = unique_dir(label);
    let _ = fs::remove_dir_all(&pack);
    copy_dir(&manifest.join("packs/oneshot-docks"), &pack);

    let mut files = Vec::new();
    markdown_files(&pack.join("cast"), &mut files);
    markdown_files(&pack.join("world"), &mut files);
    assert!(!files.is_empty(), "no markdown to strip");
    for path in &files {
        let text =
            fs::read_to_string(path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
        let stripped = spookery_kernel::lint::strip_body(&text)
            .unwrap_or_else(|err| panic!("strip {}: {err}", path.display()));
        assert!(
            !stripped.contains("Mira keeps the shop") && !stripped.contains("Bram walks"),
            "body survived in {}",
            path.display()
        );
        fs::write(path, stripped).unwrap_or_else(|err| panic!("write {}: {err}", path.display()));
    }

    let mira =
        fs::read_to_string(pack.join("cast/mira.md")).unwrap_or_else(|err| panic!("mira: {err}"));
    assert!(mira.contains(LEDGER), "frontmatter secret removed");
    let aldric = fs::read_to_string(pack.join("cast/aldric.md"))
        .unwrap_or_else(|err| panic!("aldric: {err}"));
    assert!(!aldric.contains(LEDGER), "secret appeared on aldric");

    let rev =
        spookery_kernel::canon::pack_rev(&pack).unwrap_or_else(|err| panic!("pack_rev: {err}"));
    let now_path = pack.join("state/now.yaml");
    let mut now = spookery_kernel::codec::load_now(
        &fs::read_to_string(&now_path).unwrap_or_else(|err| panic!("read now: {err}")),
    )
    .unwrap_or_else(|err| panic!("load now: {err}"));
    now.pack_rev = rev;
    let emitted =
        spookery_kernel::codec::emit_now(&now).unwrap_or_else(|err| panic!("emit now: {err}"));
    fs::write(&now_path, emitted).unwrap_or_else(|err| panic!("write now: {err}"));
    spookery_kernel::open_shard(&pack).unwrap_or_else(|err| panic!("stripped pack refused: {err}"));
    pack
}

fn saved_path(w: &World, key: &str) -> PathBuf {
    PathBuf::from(
        w.scratch
            .get(key)
            .unwrap_or_else(|| panic!("missing {key}")),
    )
}

#[given(expr = "a temp copy of packs\\/oneshot-docks with every Markdown body deleted")]
fn copy_stripped(w: &mut World) {
    w.pack = Some(strip_pack("pack"));
}

#[given(expr = "stripped Markdown bodies omit {string} and {string}")]
fn bodies_omit(w: &mut World, left: String, right: String) {
    let pack = w.pack.as_ref().unwrap();
    let mut files = Vec::new();
    markdown_files(&pack.join("cast"), &mut files);
    markdown_files(&pack.join("world"), &mut files);
    assert!(!files.is_empty(), "no markdown to strip");
    for path in &files {
        let text =
            fs::read_to_string(path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
        assert!(
            !text.contains(&left) && !text.contains(&right),
            "body survived in {}",
            path.display()
        );
    }
}

#[when(expr = "the shard runs one manual tick with no catch-up")]
fn manual_tick(w: &mut World) {
    let pack = w.pack.as_ref().unwrap();
    let mut shard = spookery_kernel::open_shard(pack).unwrap_or_else(|err| panic!("open: {err}"));
    let wall = shard.now().last_tick_at.clone();
    spookery_tick::run_tick(
        &mut shard,
        spookery_tick::Reason::Manual,
        &wall,
        spookery_tick::Catchup::none(),
    )
    .unwrap_or_else(|err| panic!("tick: {err}"));
}

#[given(expr = "a stripped start copy and played copy of packs\\/oneshot-docks")]
fn copy_stripped_replay(w: &mut World) {
    let start = strip_pack("replay-start");
    let played = unique_dir("replay-played");
    let _ = fs::remove_dir_all(&played);
    copy_dir(&start, &played);
    w.scratch
        .insert("start".to_string(), start.to_str().unwrap().to_string());
    w.scratch
        .insert("played".to_string(), played.to_str().unwrap().to_string());
    w.pack = Some(start);
}

#[when(
    expr = "the stripped played pack is kernel-ticked {int} times and replayed onto the stripped start"
)]
fn kernel_tick_and_replay(w: &mut World, n: usize) {
    let played = saved_path(w, "played");
    let start = saved_path(w, "start");
    for tick in 1..=n {
        spookery_tick::tick_pack(&played).unwrap_or_else(|err| panic!("tick {tick}: {err}"));
    }
    let golden =
        fs::read(played.join("state/now.yaml")).unwrap_or_else(|err| panic!("read golden: {err}"));
    let temp = spookery_tick::replay_recorded(&played, &start)
        .unwrap_or_else(|err| panic!("replay: {err}"));
    w.scratch.insert(
        "golden".to_string(),
        String::from_utf8(golden).unwrap_or_else(|err| panic!("golden utf-8: {err}")),
    );
    w.scratch
        .insert("replay".to_string(), temp.to_str().unwrap().to_string());
}

#[then(expr = "the stripped replay now.yaml bytes equal the played now.yaml")]
fn replay_now_matches(w: &mut World) {
    let replay = saved_path(w, "replay");
    let golden = w
        .scratch
        .get("golden")
        .unwrap_or_else(|| panic!("missing golden"));
    let replayed =
        fs::read(replay.join("state/now.yaml")).unwrap_or_else(|err| panic!("read replay: {err}"));
    assert_eq!(replayed, golden.as_bytes());
    let _ = fs::remove_dir_all(saved_path(w, "start"));
    let _ = fs::remove_dir_all(saved_path(w, "played"));
    let _ = fs::remove_dir_all(&replay);
}
