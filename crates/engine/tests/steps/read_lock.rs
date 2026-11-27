use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;

use cucumber::{given, then, when};

use spookery_engine::testing::{copy_pack, journal, lock_write, set_read_wait_handler};
use spookery_engine::{emit_save, lock};

use crate::{World, Writer};

fn hold(world: &mut World, pack: PathBuf) {
    let lock = lock::take(&pack).unwrap_or_else(|err| panic!("lock {}: {err}", pack.display()));
    let opened = spookery_engine::Session::resume(&pack, lock)
        .unwrap_or_else(|err| panic!("resume {}: {err}", pack.display()));
    world.writer = Some(Writer { session: opened });
    world.pack = Some(pack);
}

fn writer(world: &World) -> &spookery_engine::Session {
    &world
        .writer
        .as_ref()
        .expect("no writer holds the pack")
        .session
}

fn commit_round(world: &mut World) {
    let opened = &mut world
        .writer
        .as_mut()
        .expect("no writer holds the pack")
        .session;
    let wall = opened.world().last_round_at.clone();
    opened
        .play(&spookery_engine::Presence::default(), &wall)
        .unwrap_or_else(|err| panic!("play: {err}"));
}

fn peek_digest(pack: &Path) -> Result<String, String> {
    spookery_engine::Session::peek_digest(pack).map_err(|err| err.to_string())
}

/// What the status thread reports to the step, in the order it happens.
enum StatusEvent {
    /// Status found the write guard held and is waiting for it.
    Waiting,
    /// Status returned this text or error.
    Done(Result<String, String>),
}

/// Hold the write guard while the disk is `halfway`, start status, then `finish` and drop it
/// once status is waiting on the guard. `scratch` gets the text status printed.
fn peek_during(world: &mut World, halfway: impl FnOnce(), finish: impl FnOnce()) {
    let pack = world.pack().to_path_buf();
    let write = lock_write(writer(world)).unwrap_or_else(|err| panic!("write guard: {err}"));
    halfway();
    let (status_tx, status_rx) = mpsc::channel();
    let waiting_tx = status_tx.clone();
    set_read_wait_handler(&pack, move || {
        let _ = waiting_tx.send(StatusEvent::Waiting);
    });
    let peeked = thread::scope(|scope| {
        scope.spawn(|| status_tx.send(StatusEvent::Done(peek_digest(&pack))));
        let Ok(StatusEvent::Waiting) = status_rx.recv() else {
            panic!("status finished without waiting on the write guard, so it read the half-written pack");
        };
        finish();
        drop(write);
        match status_rx.recv() {
            Ok(StatusEvent::Done(peeked)) => peeked,
            _ => panic!("status sent no result"),
        }
    });
    world.scratch = peeked.unwrap_or_else(|err| panic!("status: {err}"));
}

#[given(expr = "the writer holds a docks pack")]
fn the_writer_holds_a_docks_pack(world: &mut World) {
    hold(world, copy_pack("docks", "read-lock"));
}

#[given(expr = "the writer holds a docks pack that checkpoints every round")]
fn the_writer_holds_a_checkpointing_pack(world: &mut World) {
    let pack = copy_pack("docks", "read-lock-checkpoint");
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
        writer(world).world().state_rev
    ));
    let after = fs::read(&save).unwrap();
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

/// With no write in progress, the tip is the writer's round and its text is the truth.
#[then(expr = "status prints the round the writer committed")]
fn status_prints_the_committed_round(world: &mut World) {
    let pack = world.pack();
    let ecs_world = writer(world).world();
    assert!(ecs_world.state_rev > 0, "the writer committed nothing");
    let tip = spookery_engine::Session::peek(pack).unwrap_or_else(|err| panic!("peek: {err}"));
    assert_eq!(
        emit_save(tip.world()).unwrap(),
        emit_save(ecs_world).unwrap()
    );
    let text = peek_digest(pack).unwrap_or_else(|err| panic!("status: {err}"));
    assert!(text.contains("meter "), "{text}");
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
    if let Err(err) = lock::take(world.pack()) {
        panic!("lock::take: {err}");
    }
}
