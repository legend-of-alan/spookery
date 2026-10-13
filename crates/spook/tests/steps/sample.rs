use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use cucumber::{given, then, when};
use spookery_kernel::meter;

use crate::support::{self, text_of};
use crate::World;

use super::table;

#[given(expr = "a copy of the sample-d20 pack")]
fn a_copy_of_the_sample_d20_pack(world: &mut World) {
    world.pack = Some(copy_sample("sample"));
}

#[given(expr = "a sample-d20 table")]
fn a_sample_d20_table(world: &mut World) {
    sit(world, None, true);
}

#[given(expr = "a sample-d20 table where Lark's might is {int}")]
fn a_sample_d20_table_where_larks_might_is(world: &mut World, might: usize) {
    sit(world, Some(might as i64), true);
}

#[given(expr = "a sample-d20 table where Lark has no tonic")]
fn a_sample_d20_table_where_lark_has_no_tonic(world: &mut World) {
    sit(world, None, false);
}

#[when(expr = "the player strikes the raider")]
fn the_player_strikes_the_raider(world: &mut World) {
    let (mark, added) = table::player_send("strike npc_briar", "\"kind\":\"check\"");
    world.mark = mark;
    world.player_added = added;
}

#[when(expr = "the player drinks")]
fn the_player_drinks(world: &mut World) {
    let (mark, added) = table::player_send("drink", "\"kind\":");
    world.mark = mark;
    world.player_added = added;
}

#[when(expr = "play joins the sample table")]
fn play_joins_the_sample_table(world: &mut World) {
    let port = table::free_port();
    let addr = format!("127.0.0.1:{port}");
    let pack = table::pack(world);
    table::listen(pack, &addr, port);
    let pack_arg = support::pack_arg(pack);
    world.play = Some(support::spawn(&["play", &pack_arg, "--addr", &addr]));
    wait_play(world, "Gate yard");
}

#[when(expr = "the save before the fight is kept")]
fn the_save_before_the_fight_is_kept(world: &mut World) {
    world.scratch = snapshot_of(table::pack(world)).display().to_string();
}

#[then(expr = "the raider's hp is {int}")]
fn the_raiders_hp_is(world: &mut World, hp: usize) {
    wait_hp(world, "npc_briar", hp as i64);
}

#[then(expr = "Lark's hp is {int}")]
fn larks_hp_is(world: &mut World, hp: usize) {
    wait_hp(world, "pc_lark", hp as i64);
}

#[then(expr = "the player was refused MISSING_ITEM")]
fn the_player_was_refused_missing_item(world: &mut World) {
    assert!(
        world.player_added.contains("\"code\":\"MISSING_ITEM\""),
        "{}",
        world.player_added
    );
}

#[then(expr = "play shows {int} successful rolls")]
fn play_shows_successful_rolls(world: &mut World, rolls: usize) {
    let needle = "Your roll succeeded.";
    let start = Instant::now();
    loop {
        let stdout = play_stdout(world);
        if stdout.matches(needle).count() >= rolls {
            return;
        }
        let play = world.play.as_mut().unwrap_or_else(|| panic!("no play"));
        if let Some(status) = play
            .child
            .try_wait()
            .unwrap_or_else(|err| panic!("try_wait: {err}"))
        {
            let stderr = text_of(&play.stderr);
            panic!("play exited {status:?}\nstdout:\n{stdout}\nstderr:\n{stderr}");
        }
        if start.elapsed() > Duration::from_secs(8) {
            let stderr = text_of(&world.play.as_ref().unwrap().stderr);
            panic!("play did not show {rolls} successes\nstdout:\n{stdout}\nstderr:\n{stderr}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn sit(world: &mut World, might: Option<i64>, tonic: bool) {
    let pack = copy_sample("sample-table");
    if let Some(might) = might {
        set_lark_might(&pack, might);
    }
    if !tonic {
        take_tonic(&pack);
    }
    let port = table::free_port();
    table::listen(&pack, &format!("127.0.0.1:{port}"), port);
    table::seat_actor(&pack, "pc_lark");
    world.pack = Some(pack);
}

fn copy_sample(label: &str) -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let pack = std::env::temp_dir().join(format!(
        "spook-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&pack);
    support::copy_dir(&manifest.join("../../packs/sample-d20"), &pack);
    pack
}

fn set_lark_might(pack: &Path, base: i64) {
    let path = pack.join("state/save.yaml");
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read now: {err}"));
    let from = "      might:\n        base: 40\n";
    assert_eq!(
        text.matches(from).count(),
        1,
        "Lark's might was not the only base 40"
    );
    let next = text.replacen(from, &format!("      might:\n        base: {base}\n"), 1);
    fs::write(&path, next).unwrap_or_else(|err| panic!("write now: {err}"));
}

fn take_tonic(pack: &Path) {
    let path = pack.join("state/save.yaml");
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read now: {err}"));
    let from = "    inventory:\n      - tonic\n";
    assert_eq!(text.matches(from).count(), 1, "tonic was not listed once");
    let next = text.replacen(from, "    inventory: []\n", 1);
    fs::write(&path, next).unwrap_or_else(|err| panic!("write now: {err}"));
}

fn hp_of(pack: &Path, actor_id: &str) -> i64 {
    let now = table::now_of(pack);
    let canon = spookery_kernel::canon::load_canon(pack)
        .unwrap_or_else(|err| panic!("load canon: {}", spookery_kernel::reason_of(err).token()));
    let actor = now
        .actors
        .get(actor_id)
        .unwrap_or_else(|| panic!("{actor_id} missing"));
    let def = canon
        .meters
        .get("hp")
        .unwrap_or_else(|| panic!("no hp meter"));
    let live = actor
        .meters()
        .get("hp")
        .unwrap_or_else(|| panic!("{actor_id} has no hp"));
    meter::current(def, live).unwrap_or_else(|err| panic!("hp: {}", err.token()))
}

fn wait_hp(world: &mut World, actor_id: &str, expected: i64) {
    let pack = table::pack(world).to_path_buf();
    let start = Instant::now();
    loop {
        if hp_of(&pack, actor_id) == expected {
            return;
        }
        if start.elapsed() > Duration::from_secs(5) {
            panic!(
                "{actor_id} hp is {}, not {expected}\nadded:\n{}",
                hp_of(&pack, actor_id),
                world.player_added
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn snapshot_of(pack: &Path) -> PathBuf {
    let dest = std::env::temp_dir().join(format!(
        "spook-before-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&dest);
    support::copy_dir(pack, &dest);
    dest
}

fn play_stdout(world: &World) -> String {
    text_of(
        &world
            .play
            .as_ref()
            .unwrap_or_else(|| panic!("no play"))
            .stdout,
    )
}

fn wait_play(world: &mut World, needle: &str) {
    let start = Instant::now();
    loop {
        let stdout = play_stdout(world);
        if stdout.contains(needle) {
            return;
        }
        let play = world.play.as_mut().unwrap_or_else(|| panic!("no play"));
        if let Some(status) = play
            .child
            .try_wait()
            .unwrap_or_else(|err| panic!("try_wait: {err}"))
        {
            let stderr = text_of(&play.stderr);
            panic!("play exited {status:?}\nstdout:\n{stdout}\nstderr:\n{stderr}");
        }
        if start.elapsed() > Duration::from_secs(8) {
            let stderr = text_of(&world.play.as_ref().unwrap().stderr);
            panic!("play did not show {needle}\nstdout:\n{stdout}\nstderr:\n{stderr}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
