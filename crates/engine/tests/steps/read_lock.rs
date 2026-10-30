use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;

use cucumber::{given, then, when};

use spookery_engine::testing::{journal, lock_write, on_reader_wait, temp_pack};
use spookery_engine::{emit_save, lock_pack, World as Shard};

use crate::{World, Writer};

fn hold(world: &mut World, pack: PathBuf) {
    let lock = lock_pack(&pack).unwrap_or_else(|err| panic!("lock {}: {err}", pack.display()));
    let shard =
        Shard::resume(&pack, lock).unwrap_or_else(|err| panic!("resume {}: {err}", pack.display()));
    world.writer = Some(Writer { world: shard });
    world.pack = Some(pack);
}

fn writer(world: &World) -> &Shard {
    &world
        .writer
        .as_ref()
        .expect("no writer holds the pack")
        .world
}

fn commit_round(world: &mut World) {
    let shard = &mut world
        .writer
        .as_mut()
        .expect("no writer holds the pack")
        .world;
    let wall = shard.now().last_round_at.clone();
    shard
        .play(&spookery_engine::Presence::default(), &wall)
        .unwrap_or_else(|err| panic!("play: {err}"));
}

fn peek_status(pack: &Path) -> Result<String, String> {
    Shard::peek_summary(pack).map_err(|err| err.to_string())
}

/// Hold the write guard while the disk is `halfway`, start status, then `finish` and drop it.
/// `summary` gets the order of "waited" and "peeked", and `scratch` the text status printed.
fn peek_during(world: &mut World, halfway: impl FnOnce(), finish: impl FnOnce()) {
    let pack = world.pack().to_path_buf();
    let (tx, rx) = mpsc::channel();
    on_reader_wait(tx.clone());
    let write = lock_write(writer(world)).unwrap_or_else(|err| panic!("write guard: {err}"));
    halfway();
    let peeked = thread::scope(|scope| {
        let status = scope.spawn(|| {
            let peeked = peek_status(&pack);
            let _ = tx.send("peeked");
            peeked
        });
        let first = rx.recv().expect("status ended without a word");
        finish();
        drop(write);
        let peeked = status.join().expect("status panicked");
        world.summary = vec![first.to_string()];
        world.summary.extend(rx.try_iter().map(str::to_string));
        peeked
    });
    world.scratch = peeked.unwrap_or_else(|err| panic!("status: {err}"));
}

#[given(expr = "the writer holds a docks pack")]
fn the_writer_holds_a_docks_pack(world: &mut World) {
    hold(world, temp_pack("read-lock"));
}

#[given(expr = "the writer holds a docks pack that checkpoints every round")]
fn the_writer_holds_a_checkpointing_pack(world: &mut World) {
    let pack = temp_pack("read-lock-checkpoint");
    let path = pack.join("pack.yaml");
    let mut text = fs::read_to_string(&path).unwrap();
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str("checkpoint_every: 1\n");
    fs::write(&path, text).unwrap();
    hold(world, pack);
}

#[when(expr = "status peeks while the writer is halfway through appending a round")]
fn status_peeks_mid_append(world: &mut World) {
    let pack = world.pack().to_path_buf();
    let before = journal::find_live_bundle(&pack)
        .map(|path| fs::read(path).unwrap().len())
        .unwrap_or(0);
    commit_round(world);
    let bundle = journal::find_live_bundle(&pack).expect("the round is in a live bundle");
    let full = fs::read(&bundle).unwrap();
    let torn = &full[..before + (full.len() - before) / 2];
    peek_during(
        world,
        || fs::write(&bundle, torn).unwrap(),
        || fs::write(&bundle, &full).unwrap(),
    );
}

#[when(expr = "status peeks while the writer is halfway through publishing a checkpoint")]
fn status_peeks_mid_checkpoint(world: &mut World) {
    let pack = world.pack().to_path_buf();
    let save = pack.join("state/save.yaml");
    let before = fs::read(&save).unwrap();
    commit_round(world);
    let snap = pack.join(format!(
        "state/checkpoints/{}.yaml",
        writer(world).now().state_rev
    ));
    let after = fs::read(&save).unwrap();
    // The head is published and the save and snapshot are not yet.
    peek_during(
        world,
        || {
            fs::write(&save, &before).unwrap();
            fs::remove_file(&snap).unwrap();
        },
        || {
            fs::write(&save, &after).unwrap();
            fs::write(&snap, &after).unwrap();
        },
    );
}

#[then(expr = "status waited for the write to finish")]
fn status_waited(world: &mut World) {
    assert_eq!(world.summary, ["waited", "peeked"]);
}

/// With no write in progress, the tip is the writer's round and its text is the truth.
#[then(expr = "status prints the round the writer committed")]
fn status_prints_the_committed_round(world: &mut World) {
    let pack = world.pack();
    let now = writer(world).now();
    assert!(now.state_rev > 0, "the writer committed nothing");
    let tip = Shard::peek(pack).unwrap_or_else(|err| panic!("peek: {err}"));
    assert_eq!(emit_save(tip.now()).unwrap(), emit_save(now).unwrap());
    let text = peek_status(pack).unwrap_or_else(|err| panic!("status: {err}"));
    assert!(text.contains("clock "), "{text}");
    assert_eq!(world.scratch, text);
}

#[given(expr = "a second handle to the lock file stays open, like a child that has not exec'd yet")]
fn a_second_handle_stays_open(world: &mut World) {
    world.lock_clone = Some(spookery_engine::testing::clone_lock_file(writer(world)));
}

#[when(expr = "the writer drops its lock")]
fn the_writer_drops_its_lock(world: &mut World) {
    world.writer = None;
}

#[then(expr = "a new writer can lock the pack")]
fn a_new_writer_can_lock_the_pack(world: &mut World) {
    if let Err(err) = lock_pack(world.pack()) {
        panic!("lock_pack: {err}");
    }
}
