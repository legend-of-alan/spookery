use std::fs;
#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use spookery_tick::{replay_recorded, run_tick, tick_pack, Catchup, Reason};

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

fn stripped_pack(label: &str) -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixture = manifest.join("packs/oneshot-docks");
    let pack = std::env::temp_dir().join(format!(
        "spook-strip-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&pack);
    copy_dir(&fixture, &pack);

    let mut files = Vec::new();
    markdown_files(&pack.join("cast"), &mut files);
    markdown_files(&pack.join("world"), &mut files);
    assert!(!files.is_empty(), "no markdown to strip");
    for path in &files {
        let text = fs::read_to_string(path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
        let stripped = spookery_kernel::lint::strip_body(&text)
            .unwrap_or_else(|err| panic!("strip {}: {err}", path.display()));
        assert!(
            !stripped.contains("Mira keeps the shop") && !stripped.contains("Bram walks"),
            "body survived in {}",
            path.display()
        );
        fs::write(path, stripped).unwrap_or_else(|err| panic!("write {}: {err}", path.display()));
    }

    let mira = fs::read_to_string(pack.join("cast/mira.md")).unwrap_or_else(|err| panic!("mira: {err}"));
    assert!(mira.contains(LEDGER), "frontmatter secret removed");
    let aldric = fs::read_to_string(pack.join("cast/aldric.md")).unwrap_or_else(|err| panic!("aldric: {err}"));
    assert!(!aldric.contains(LEDGER), "secret appeared on aldric");

    let rev = spookery_kernel::canon::pack_rev(&pack).unwrap_or_else(|err| panic!("pack_rev: {err}"));
    let now_path = pack.join("state/now.yaml");
    let mut now = spookery_kernel::codec::load_now(
        &fs::read_to_string(&now_path).unwrap_or_else(|err| panic!("read now: {err}")),
    )
    .unwrap_or_else(|err| panic!("load now: {err}"));
    now.pack_rev = rev;
    let emitted = spookery_kernel::codec::emit_now(&now).unwrap_or_else(|err| panic!("emit now: {err}"));
    fs::write(&now_path, emitted).unwrap_or_else(|err| panic!("write now: {err}"));
    spookery_kernel::open_shard(&pack).unwrap_or_else(|err| panic!("stripped pack refused: {err}"));
    pack
}

fn tick_ok(pack: &Path, tick: u32) {
    let output = spook(&["tick", pack.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "tick {tick}\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn product_11_strip_bodies() {
    let closed = stripped_pack("closed");
    for tick in 1..=100 {
        tick_ok(&closed, tick);
    }
    let perceive = {
        let shard = spookery_kernel::open_shard(&closed).unwrap_or_else(|err| panic!("open: {err}"));
        let view = spookery_kernel::perceive(&shard, "npc:aldric").unwrap_or_else(|err| panic!("{err}"));
        format!("{view:?}")
    };
    let memories = fs::read_to_string(closed.join("state/memories/aldric.jsonl")).unwrap_or_default();
    let rumors = format!(
        "{}\n{}",
        fs::read_to_string(closed.join("state/rumors.yaml")).unwrap_or_default(),
        fs::read_to_string(closed.join("state/now.yaml")).unwrap_or_default()
    );
    assert!(!perceive.contains(LEDGER), "ledger sentence in Aldric's perceive");
    assert!(!memories.contains(LEDGER), "ledger sentence in Aldric's memories");
    assert!(!rumors.contains(LEDGER), "ledger sentence in rumors");
    let _ = fs::remove_dir_all(&closed);

    let paused = stripped_pack("paused");
    for tick in 1..=12 {
        tick_ok(&paused, tick);
    }
    let shard = spookery_kernel::open_shard(&paused).unwrap_or_else(|err| panic!("open: {err}"));
    assert_eq!(shard.now().tick_index, 12, "twelve ticks commit");
    let clock = shard
        .now()
        .clocks
        .get("assassination_debt")
        .unwrap_or_else(|| panic!("assassination_debt missing"));
    assert_eq!(clock.value, 2);
    assert_eq!(clock.max, 6);
    assert!(clock.paused);
    assert!(clock.requires_presence);
    assert_eq!(clock.owners, vec!["pc_rowan".to_string()]);
    drop(shard);
    let _ = fs::remove_dir_all(&paused);

    let crashed = stripped_pack("crash");
    let before = spookery_kernel::open_shard(&crashed).unwrap_or_else(|err| panic!("open: {err}"));
    let state_rev = before.now().state_rev;
    let lamp_oil = before
        .now()
        .places
        .get("docks_shop")
        .and_then(|place| place.stock.get("lamp_oil"))
        .copied()
        .unwrap_or_else(|| panic!("lamp_oil missing"));
    let proposals = before.now().proposals.clone();
    drop(before);
    let output = Command::new(env!("CARGO_BIN_EXE_spook"))
        .args(["tick", crashed.to_str().unwrap()])
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
    assert!(!crashed.join("state/.txn").exists(), "apply_effect created state/.txn");
    let restarted = spookery_kernel::open_shard(&crashed).unwrap_or_else(|err| panic!("restart: {err}"));
    assert_eq!(restarted.now().state_rev, state_rev, "state_rev changed");
    assert_eq!(
        restarted
            .now()
            .places
            .get("docks_shop")
            .and_then(|place| place.stock.get("lamp_oil"))
            .copied(),
        Some(lamp_oil),
        "stock value changed"
    );
    assert_eq!(restarted.now().proposals, proposals, "proposal list changed");
    assert!(!crashed.join("state/proposals").exists(), "a proposal file appeared");
    assert!(!crashed.join("state/memories").exists(), "a memory file appeared");
    let _ = fs::remove_dir_all(&crashed);

    let shop = stripped_pack("shop");
    let rate = {
        let shard = spookery_kernel::open_shard(&shop).unwrap_or_else(|err| panic!("open: {err}"));
        shard
            .canon()
            .profile("mira")
            .and_then(|profile| profile.stock_rate.get("docks_shop"))
            .and_then(|rates| rates.get("lamp_oil"))
            .copied()
            .unwrap_or_else(|| panic!("stock_rate missing"))
    };
    assert_eq!(rate, 4);
    let mut previous = lamp_oil_of(&shop);
    assert!(previous >= 0);
    for tick in 1..=13 {
        tick_ok(&shop, tick);
        let qty = lamp_oil_of(&shop);
        let delta = qty - previous;
        assert!(delta.abs() <= rate, "tick {tick}: lamp_oil {previous} -> {qty}");
        assert!(qty >= 0, "tick {tick}: lamp_oil {qty}");
        previous = qty;
    }
    let _ = fs::remove_dir_all(&shop);

    let busy = stripped_pack("busy");
    {
        let shard = spookery_kernel::open_shard(&busy).unwrap_or_else(|err| panic!("open: {err}"));
        let mut now = shard.now().clone();
        drop(shard);
        now.presence.humans = 1;
        now.presence.actors_present = vec!["pc_rowan".to_string()];
        now.world_time = 360;
        *now.actors
            .get_mut("mira")
            .unwrap_or_else(|| panic!("mira missing"))
            .place_mut() = "docks_shop".to_string();
        let text = spookery_kernel::codec::emit_now(&now).unwrap_or_else(|err| panic!("emit: {err}"));
        fs::write(busy.join("state/now.yaml"), text).unwrap();
    }
    {
        let mut shard = spookery_kernel::open_shard(&busy).unwrap_or_else(|err| panic!("open: {err}"));
        let wall = shard.now().last_tick_at.clone();
        run_tick(&mut shard, Reason::Manual, &wall, Catchup::none()).unwrap_or_else(|err| panic!("tick: {err}"));
        assert_eq!(shard.now().actors.get("mira").unwrap().place(), "docks_shop");
        assert_eq!(shard.now().actors.get("mira").unwrap().tick().unwrap().last_act_tick, 0);
        assert_eq!(shard.now().actors.get("bram").unwrap().place(), "docks_shop");
        assert_eq!(shard.now().actors.get("bram").unwrap().tick().unwrap().last_act_tick, 1);
    }
    let _ = fs::remove_dir_all(&busy);

    let replay_start = stripped_pack("replay");
    let replay_played = std::env::temp_dir().join(format!(
        "spook-strip-replay-played-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&replay_played);
    copy_dir(&replay_start, &replay_played);
    for tick in 1..=20 {
        tick_pack(&replay_played).unwrap_or_else(|err| panic!("tick {tick}: {err}"));
    }
    let golden = fs::read(replay_played.join("state/now.yaml")).unwrap();
    let temp = replay_recorded(&replay_played, &replay_start).unwrap_or_else(|err| panic!("replay: {err}"));
    assert_eq!(fs::read(temp.join("state/now.yaml")).unwrap(), golden);
    let _ = fs::remove_dir_all(&replay_start);
    let _ = fs::remove_dir_all(&replay_played);
    let _ = fs::remove_dir_all(&temp);
}

fn lamp_oil_of(pack: &Path) -> i64 {
    let shard = spookery_kernel::open_shard(pack).unwrap_or_else(|err| panic!("open: {err}"));
    shard
        .now()
        .places
        .get("docks_shop")
        .and_then(|place| place.stock.get("lamp_oil"))
        .copied()
        .unwrap_or_else(|| panic!("lamp_oil missing"))
}
