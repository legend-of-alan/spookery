use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;

use cucumber::{given, then, when};

use spookery_engine::testing::{journal, on_reader_wait, temp_pack, write_guard};
use spookery_engine::{emit_save, lock_pack, World as Shard};

use crate::{World, Writer};

fn hold(world: &mut World, pack: PathBuf) {
    let lock = lock_pack(&pack).unwrap_or_else(|err| panic!("lock {}: {err}", pack.display()));
    let shard = Shard::resume(&pack, &lock)
        .unwrap_or_else(|err| panic!("resume {}: {err}", pack.display()));
    world.writer = Some(Writer { world: shard, lock });
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
        .play(&wall)
        .unwrap_or_else(|err| panic!("play: {err}"));
}

fn peek_save(pack: &Path) -> Result<String, String> {
    let shard = Shard::peek(pack).map_err(|err| err.to_string())?;
    emit_save(shard.now()).map_err(|err| err.to_string())
}

/// Hold the write guard while the disk is `halfway`, start status, then `finish` and drop it.
/// `summary` gets the order of "waited" and "peeked", and `scratch` the save status built.
fn peek_during(world: &mut World, halfway: impl FnOnce(), finish: impl FnOnce()) {
    let pack = world.pack().to_path_buf();
    let (tx, rx) = mpsc::channel();
    on_reader_wait(tx.clone());
    let write = write_guard(&pack).unwrap_or_else(|err| panic!("write guard: {err}"));
    halfway();
    let peeked = thread::scope(|scope| {
        let status = scope.spawn(|| {
            let peeked = peek_save(&pack);
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

#[given(expr = "the writer has committed a round")]
fn the_writer_has_committed_a_round(world: &mut World) {
    commit_round(world);
}

#[when(expr = "status peeks the pack")]
fn status_peeks_the_pack(world: &mut World) {
    world.scratch = peek_save(world.pack()).unwrap_or_else(|err| panic!("status: {err}"));
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

#[when(expr = "a second writer locks the pack")]
fn a_second_writer_locks_the_pack(world: &mut World) {
    world.scratch = match lock_pack(world.pack()) {
        Ok(_) => "locked".to_string(),
        Err(err) => err.to_string(),
    };
}

#[then(expr = "status waited for the write to finish")]
fn status_waited(world: &mut World) {
    assert_eq!(world.summary, ["waited", "peeked"]);
}

#[then(expr = "status reports the round the writer committed")]
fn status_reports_the_committed_round(world: &mut World) {
    let now = writer(world).now();
    assert!(now.state_rev > 0, "the writer committed nothing");
    assert_eq!(world.scratch, emit_save(now).unwrap());
}

#[then(expr = "the save, head, snapshot and journal checkpoint all hold that state_rev")]
fn the_checkpoint_set_matches(world: &mut World) {
    let pack = world.pack();
    let now = writer(world).now();
    let rev = now.state_rev;
    let save = fs::read(pack.join("state/save.yaml")).unwrap();
    assert_eq!(save, emit_save(now).unwrap().into_bytes());
    let snap = fs::read(pack.join(format!("state/checkpoints/{rev}.yaml"))).unwrap();
    assert_eq!(snap, save);
    let head = fs::read_to_string(pack.join("state/journal/head.json")).unwrap();
    assert!(head.contains(&format!("\"state_rev\":{rev},")), "{head}");
    let entries = journal::read_entries(pack).unwrap_or_else(|fault| panic!("{}", fault.detail));
    let mark = journal::latest_checkpoint(&entries).expect("a journal checkpoint");
    assert_eq!(mark.state_rev, rev);
    assert_eq!(mark.state_hash, journal::hash_now(&save));
}

#[then(expr = "the second writer is refused with COMMIT_CONFLICT")]
fn the_second_writer_is_refused(world: &mut World) {
    assert_eq!(world.scratch, "COMMIT_CONFLICT");
}
