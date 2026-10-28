use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use cucumber::{given, then, when};

use crate::support::{self, text_of};
use crate::World;

use super::common::play_stdout;
use super::table;

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
    let (mark, added) = table::player_send(world, "strike npc_briar", "\"kind\":\"check\"");
    world.mark = mark;
    world.player_added = added;
}

#[when(expr = "the player waits {int} and strikes the raider")]
fn the_player_waits_and_strikes(world: &mut World, times: usize) {
    for _ in 0..times {
        table::player_send(world, "wait", "\"kind\":\"perceive\"");
    }
    let (mark, added) = table::player_send(world, "strike npc_briar", "\"kind\":\"check\"");
    world.mark = mark;
    world.player_added = added;
}

#[then(expr = "the roll is {string}")]
fn the_roll_is(world: &mut World, band: String) {
    let needle = format!("\"outcome\":\"{band}\"");
    assert!(
        world.player_added.contains(&needle),
        "expected {band}\n{}",
        world.player_added
    );
}

#[when(expr = "the player drinks")]
fn the_player_drinks(world: &mut World) {
    let (mark, added) = table::player_send(world, "use tonic", "\"kind\":");
    world.mark = mark;
    world.player_added = added;
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
    let pack = support::copy_pack("sample-d20", "sample-table");
    if let Some(might) = might {
        set_lark_might(&pack, might);
    }
    if !tonic {
        take_tonic(&pack);
    }
    table::listen(world, &pack);
    table::seat_actor(world, &pack, "pc_lark");
    world.pack = Some(pack);
}

fn set_lark_might(pack: &Path, base: i64) {
    let path = pack.join("state/save.yaml");
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read now: {err}"));
    let from = "      might:\n        base: 30\n";
    assert_eq!(
        text.matches(from).count(),
        1,
        "Lark's might was not the only base 30"
    );
    let next = text.replacen(from, &format!("      might:\n        base: {base}\n"), 1);
    fs::write(&path, next).unwrap_or_else(|err| panic!("write now: {err}"));
}

fn take_tonic(pack: &Path) {
    let path = pack.join("state/save.yaml");
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read now: {err}"));
    let from = "    inventory:\n      tonic: 1\n";
    assert_eq!(text.matches(from).count(), 1, "tonic was not listed once");
    let next = text.replacen(from, "    inventory: {}\n", 1);
    fs::write(&path, next).unwrap_or_else(|err| panic!("write now: {err}"));
}

fn hp_of(pack: &Path, actor_id: &str) -> i64 {
    let now = table::now_of(pack);
    let canon = spookery_engine::load_canon(pack)
        .unwrap_or_else(|err| panic!("load canon: {}", spookery_engine::reason_of(err).token()));
    now.meter_value(&canon, actor_id, "hp")
        .unwrap_or_else(|err| panic!("hp: {}", err.token()))
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
    let dest = support::fresh_dir("before");
    support::copy_dir(pack, &dest);
    dest
}
