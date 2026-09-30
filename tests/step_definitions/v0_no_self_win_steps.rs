use std::fs;
use std::path::PathBuf;

use cucumber::{given, then, when};

use crate::support::{copy_dir, spook};
use crate::World;

#[given(expr = "a temp copy of pack packs\\/oneshot-docks")]
fn copy_docks(w: &mut World) {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let pack = std::env::temp_dir().join(format!(
        "spook-bdd-t02-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&pack);
    copy_dir(&manifest.join("packs/oneshot-docks"), &pack);
    w.pack = Some(pack);
}

#[when(expr = "the pack is ticked {int} times")]
fn tick_n(w: &mut World, n: usize) {
    let pack = w.pack.as_ref().unwrap();
    for _ in 0..n {
        w.outputs.push(spook(&["tick", pack.to_str().unwrap()]));
    }
}

#[then(expr = "every tick exits 0")]
fn ticks_ok(w: &mut World) {
    for output in &w.outputs {
        assert!(
            output.status.success(),
            "stdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[then(expr = "tick_index is {int}")]
fn tick_index_is(w: &mut World, n: u64) {
    let shard = spookery_kernel::open_shard(w.pack.as_ref().unwrap()).unwrap();
    assert_eq!(shard.now().tick_index, n);
}

#[then(expr = "clock {word} has value {int} and max {int}")]
fn clock_is(w: &mut World, name: String, value: i64, max: i64) {
    let shard = spookery_kernel::open_shard(w.pack.as_ref().unwrap()).unwrap();
    let clock = shard.now().clocks.get(&name).unwrap();
    assert_eq!(clock.value, value);
    assert_eq!(clock.max, max);
}

#[then(expr = "clock {word} is paused")]
fn clock_paused(w: &mut World, name: String) {
    let shard = spookery_kernel::open_shard(w.pack.as_ref().unwrap()).unwrap();
    let clock = shard.now().clocks.get(&name).unwrap();
    assert!(clock.paused);
}

#[then(expr = "clock {word} requires presence")]
fn clock_requires_presence(w: &mut World, name: String) {
    let shard = spookery_kernel::open_shard(w.pack.as_ref().unwrap()).unwrap();
    let clock = shard.now().clocks.get(&name).unwrap();
    assert!(clock.requires_presence);
}

#[then(expr = "clock {word} owners are {word}")]
fn clock_owners(w: &mut World, name: String, owner: String) {
    let shard = spookery_kernel::open_shard(w.pack.as_ref().unwrap()).unwrap();
    let clock = shard.now().clocks.get(&name).unwrap();
    assert_eq!(clock.owners, vec![owner]);
}
