//! Steps used by more than one cucumber program.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};

use cucumber::{given, then, when};
use spookery_kernel::api::model::Now;
use spookery_kernel::api::store::codec::{emit_save, load_save};

use super::table;
use crate::support::{self, temp_pack};
use crate::World;

pub(crate) const EVENING: u64 = 21 * 60 + 30;

pub(crate) fn edit_save(pack: &Path, edit: impl FnOnce(&mut Now)) {
    let path = pack.join("state/save.yaml");
    let mut now = load_save(&fs::read_to_string(&path).unwrap()).unwrap();
    edit(&mut now);
    now.pack_rev = spookery_kernel::api::canon::pack_rev(pack).unwrap();
    fs::write(&path, emit_save(&now).unwrap()).unwrap();
}

pub(crate) fn give_key(now: &mut Now) {
    now.actors
        .get_mut("pc_rowan")
        .unwrap()
        .inventory_mut()
        .insert("brass_key".to_string(), 1);
}

pub(crate) fn add_trapdoor(pack: &Path) {
    let path = pack.join("world/places/docks_shop.md");
    let text = fs::read_to_string(&path).unwrap();
    let next = text.replace(
        "    hidden: false\n",
        "    hidden: false\n  - id: fact_trapdoor\n    text: A trapdoor is under the rug.\n    hidden: true\n",
    );
    let next = next.replace(
        "    locked_text: The stair door is locked.\n",
        "    locked_text: The stair door is locked.\n  - to: cellar\n    name: trapdoor\n    hidden: true\n    revealed_by: fact_trapdoor\n",
    );
    assert_ne!(text, next);
    fs::write(&path, next).unwrap();
    fs::write(
        pack.join("world/places/cellar.md"),
        "---\nschema: place\nid: cellar\ndisplay_name: Cellar\nfacts: []\nexits: []\n---\nUnder the shop.\n",
    )
    .unwrap();
}

pub(crate) fn run_check_json(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::run(
        world,
        &["check", "--json", &crate::support::pack_arg(&pack)],
    );
}

pub(crate) fn replace_file(path: &Path, from: &str, to: &str) {
    let text = fs::read_to_string(path).unwrap();
    assert!(text.contains(from), "missing {from} in {}", path.display());
    fs::write(path, text.replacen(from, to, 1)).unwrap();
}

fn break_pack(pack: &Path, fault: &str) {
    match fault {
        "missing when" => replace_file(
            &pack.join("rules/verbs.yaml"),
            "  - id: grit\n    when:\n      verb: grit\n",
            "  - id: grit\n",
        ),
        "unknown field" => replace_file(
            &pack.join("rules/verbs.yaml"),
            "  - id: grit\n    when:\n",
            "  - id: grit\n    audiance: player\n    when:\n",
        ),
        "unknown check" => replace_file(
            &pack.join("rules/verbs.yaml"),
            "    check: grit\n",
            "    check: courage\n",
        ),
        "duplicate move" => replace_file(
            &pack.join("rules/verbs.yaml"),
            "  - id: wait\n",
            "  - id: grit\n",
        ),
        "old format tag" => replace_file(
            &pack.join("rules/verbs.yaml"),
            "schema: verbs\n",
            &format!("schema: verbs/{}\n", "v0"),
        ),
        "missing meters" => {
            fs::remove_file(pack.join("rules/meters.yaml")).unwrap();
        }
        "missing display_name" => {
            replace_file(&pack.join("cast/pc-rowan.md"), "display_name: Rowan\n", "")
        }
        "pack_rev mismatch" => {
            let path = pack.join("state/save.yaml");
            let text = fs::read_to_string(&path).unwrap();
            let text = text.replacen("pack_rev: blake2s:", "pack_rev: blake2s:0", 1);
            // The original hash does not start with an extra 0 in a way that stays equal:
            // inserting one 0 changes the stored hash.
            fs::write(path, text).unwrap();
        }
        "hidden fact in a place body" => {
            let path = pack.join("world/places/docks_shop.md");
            let mut text = fs::read_to_string(&path).unwrap();
            text.push_str("The lamp-oil ledger is short.\n");
            fs::write(path, text).unwrap();
        }
        "hidden fact in a message" => replace_file(
            &pack.join("cast/mira.md"),
            "text: Back at dusk. Oil on the shelf.\n",
            "text: The lamp-oil ledger is short.\n",
        ),
        "exit to an unknown place" => replace_file(
            &pack.join("world/places/docks_shop.md"),
            "  - to: docks_street\n",
            "  - to: nowhere\n",
        ),
        "duplicate exit" => replace_file(
            &pack.join("world/places/docks_shop.md"),
            "    name: docks_street\n",
            "    name: docks_street\n  - to: docks_street\n    name: docks_street\n",
        ),
        "idle maybe" => replace_file(
            &pack.join("rules/tick.yaml"),
            "schema: tick\n",
            "schema: tick\nidle: maybe\n",
        ),
        "zero tick interval" => replace_file(
            &pack.join("rules/tick.yaml"),
            "tick_every_real_seconds: 300\n",
            "tick_every_real_seconds: 0\n",
        ),
        "zero idle minutes" => replace_file(
            &pack.join("rules/tick.yaml"),
            "idle_minutes_per_tick: 60\n",
            "idle_minutes_per_tick: 0\n",
        ),
        other => panic!("unknown fault {other}"),
    }
}

pub(crate) fn join_play(world: &mut World, raw: bool) {
    let pack = table::pack(world).to_path_buf();
    table::listen(world, &pack);
    let addr = table::game_addr(world);
    let pack_arg = support::pack_arg(&pack);
    let mut args = vec!["play".to_string(), pack_arg, "--addr".to_string(), addr];
    if raw {
        args.push("--raw".to_string());
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    world.play = Some(support::spawn(&refs));
    let needle = if raw {
        "\"kind\":\"perceive\""
    } else {
        "You are in"
    };
    wait_play(world, needle);
}

pub(crate) fn wait_play(world: &mut World, needle: &str) {
    wait_play_until(world, |stdout| stdout.contains(needle), needle);
}

/// Poll play stdout until `ready` is true, or panic on exit / timeout.
pub(crate) fn wait_play_until(world: &mut World, ready: impl Fn(&str) -> bool, label: &str) {
    let start = Instant::now();
    loop {
        let stdout = play_stdout(world);
        if ready(&stdout) {
            return;
        }
        let play = world.play.as_mut().unwrap_or_else(|| panic!("no play"));
        if let Some(status) = play
            .child
            .try_wait()
            .unwrap_or_else(|err| panic!("try_wait: {err}"))
        {
            let stderr = support::text_of(&play.stderr);
            panic!("play exited {status:?}\nstdout:\n{stdout}\nstderr:\n{stderr}");
        }
        if start.elapsed() > Duration::from_secs(8) {
            let stderr = world
                .play
                .as_ref()
                .map(|play| support::text_of(&play.stderr))
                .unwrap_or_default();
            panic!("missing {label}\nstdout:\n{stdout}\nstderr:\n{stderr}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub(crate) fn play_stdout(world: &mut World) -> String {
    let play = world.play.as_ref().unwrap_or_else(|| panic!("no play"));
    let text = support::text_of(&play.stdout);
    world.stdout = text.clone();
    text
}

#[given(expr = "a docks table")]
fn a_docks_table(world: &mut World) {
    let pack = crate::support::temp_pack("status");
    world.before = table::files(&pack);
    world.pack = Some(pack);
}

#[then(expr = "the pack bytes are unchanged")]
fn the_pack_bytes_are_unchanged(world: &mut World) {
    let pack = world
        .pack
        .as_ref()
        .unwrap_or_else(|| panic!("no docks table"));
    assert_eq!(
        table::files(pack),
        world.before,
        "the command wrote the pack"
    );
}

#[given(expr = "a docks table with a seated GM")]
fn a_docks_table_with_a_seated_gm(world: &mut World) {
    let pack = temp_pack("gm-seat");
    let opened = fs::read(pack.join("state/save.yaml"))
        .unwrap_or_else(|err| panic!("read save.yaml: {err}"));
    table::listen(world, &pack);
    table::seat_gm(world);
    table::live_mut(world).opened_now = opened;
    world.pack = Some(pack);
}

#[given(expr = "a docks table with a seated player")]
fn a_docks_table_with_a_seated_player(world: &mut World) {
    let pack = temp_pack("speech");
    table::listen(world, &pack);
    table::seat_player(world, &pack);
    world.pack = Some(pack);
}

#[when(expr = "the operator runs spook check")]
fn the_operator_runs_spook_check(world: &mut World) {
    let pack = table::pack(world);
    let output = support::spook(&["check", &crate::support::pack_arg(pack)]);
    world.code = output.status.code();
    world.stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    world.stderr = String::from_utf8_lossy(&output.stderr).into_owned();
}

#[when(expr = "the operator runs spook check --json")]
fn the_operator_runs_spook_check_json(world: &mut World) {
    run_check_json(world);
}

#[when(expr = "the copy is broken by {string}")]
fn the_copy_is_broken_by(world: &mut World, fault: String) {
    let pack = table::pack(world).to_path_buf();
    break_pack(&pack, &fault);
    world.before = table::files(&pack);
}

#[then(expr = "the check exits 0")]
fn the_check_exits_0(world: &mut World) {
    assert_eq!(world.code, Some(0), "{}\n{}", world.stdout, world.stderr);
}

#[then(expr = "the check exits 1")]
fn the_check_exits_1(world: &mut World) {
    assert_eq!(world.code, Some(1), "{}\n{}", world.stdout, world.stderr);
}

#[then(expr = "stdout says {string}")]
fn stdout_says(world: &mut World, text: String) {
    assert_eq!(world.stdout.trim(), text, "{}", world.stdout);
}

#[then(expr = "the report says {string}")]
fn the_report_says(world: &mut World, text: String) {
    assert!(
        world.stderr.contains(&text),
        "missing {text}\n{}",
        world.stderr
    );
}

#[when(expr = "play joins the table on a free loopback address")]
fn play_joins_the_table_on_a_free_loopback_address(world: &mut World) {
    join_play(world, false);
}

#[when(expr = "play types {string}")]
fn play_types(world: &mut World, line: String) {
    let play = world.play.as_mut().unwrap_or_else(|| panic!("no play"));
    let stdin = play
        .stdin
        .as_mut()
        .unwrap_or_else(|| panic!("play stdin is closed"));
    writeln!(stdin, "{line}").unwrap_or_else(|err| panic!("type: {err}"));
    stdin.flush().unwrap_or_else(|err| panic!("flush: {err}"));
}

#[then(expr = "play shows {string}")]
fn play_shows(world: &mut World, text: String) {
    wait_play(world, &text);
}

#[then(expr = "play shows no protocol JSON")]
fn play_shows_no_protocol_json(world: &mut World) {
    let stdout = play_stdout(world);
    assert!(!stdout.contains('{'), "{stdout}");
    assert!(!stdout.contains("\"kind\""), "{stdout}");
}

#[then(expr = "the command exits 0")]
fn the_command_exits_0(world: &mut World) {
    assert_eq!(
        world.code,
        Some(0),
        "stdout:\n{}\nstderr:\n{}",
        world.stdout,
        world.stderr
    );
}

#[then(expr = "the command exits 1")]
fn the_command_exits_1(world: &mut World) {
    assert_eq!(
        world.code,
        Some(1),
        "stdout:\n{}\nstderr:\n{}",
        world.stdout,
        world.stderr
    );
}

#[then(expr = "stderr is empty")]
fn stderr_is_empty(world: &mut World) {
    assert!(world.stderr.is_empty(), "{}", world.stderr);
}

#[then(expr = "stderr is NOT_IN_SPEC")]
fn stderr_is_not_in_spec(world: &mut World) {
    assert_eq!(world.stderr.trim(), "NOT_IN_SPEC", "{}", world.stdout);
}

#[then(expr = "stdout is empty")]
fn stdout_is_empty(world: &mut World) {
    assert!(world.stdout.is_empty(), "{}", world.stdout);
}

#[then(expr = "world_time is 1080")]
fn world_time_is_1080(world: &mut World) {
    assert_eq!(table::now_of(table::pack(world)).world_time, 1080);
}

#[then(expr = "world_time is 1085")]
fn world_time_is_1085(world: &mut World) {
    table::wait_world_time(table::pack(world), 1085);
}

#[when(expr = "the player waits")]
fn the_player_waits(world: &mut World) {
    let (mark, added) = table::player_send(world, "wait", "\"kind\":");
    world.mark = mark;
    world.player_added = added;
}

#[when(expr = "the GM commits the round")]
fn the_gm_commits_the_round(world: &mut World) {
    world.stdout = table::gm_call(world, 2, "commit_round", r#"{"include_tick":true}"#);
}

#[then(expr = "a tick file was written")]
fn a_tick_file_was_written(world: &mut World) {
    assert_eq!(
        table::list_tick_keys(table::pack(world)),
        vec!["tick-0001".to_string()]
    );
}

#[then(expr = "no tick file was written")]
fn no_tick_file(world: &mut World) {
    assert!(table::list_tick_keys(table::pack(world)).is_empty());
}

#[then(expr = "save.yaml is unchanged since the seat")]
fn now_unchanged_since_the_seat(world: &mut World) {
    let seated = table::live_mut(world).seated_now.clone();
    let now = fs::read(table::pack(world).join("state/save.yaml"))
        .unwrap_or_else(|err| panic!("read save.yaml: {err}"));
    assert_eq!(now, seated);
}

#[when(expr = "the operator runs replay")]
fn the_operator_runs_replay(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::run(world, &["replay", pack.to_str().unwrap()]);
}

#[given(expr = "the trapdoor is hidden and rowan does not know fact_trapdoor")]
fn the_trapdoor_is_hidden(world: &mut World) {
    let pack = temp_pack("trapdoor");
    add_trapdoor(&pack);
    edit_save(&pack, |_| {});
    table::listen(world, &pack);
    table::seat_player(world, &pack);
    world.pack = Some(pack);
    world.player_added = table::player_text(world);
}

#[then(expr = "rowan's perceive and help do not list the trapdoor")]
fn perceive_and_help_hide_the_trapdoor(world: &mut World) {
    let perceive = table::player_text(world);
    assert!(
        !perceive.contains("trapdoor"),
        "perceive listed it\n{perceive}"
    );
    let (_mark, help) = table::player_send(world, "help", "Exits:");
    assert!(!help.contains("trapdoor"), "help listed it\n{help}");
    assert!(help.contains("stairs"), "help hid the stairs\n{help}");
}

#[given(expr = "the stairs exit costs 10 minutes")]
fn the_stairs_cost_ten_minutes(world: &mut World) {
    let pack = temp_pack("ten-minutes");
    let path = pack.join("world/places/docks_shop.md");
    let text = fs::read_to_string(&path)
        .unwrap()
        .replace("time: 2m", "time: 10m");
    fs::write(&path, text).unwrap();
    edit_save(&pack, |now| {
        give_key(now);
        now.world_time = EVENING;
    });
    table::listen(world, &pack);
    table::seat_player(world, &pack);
    world.pack = Some(pack);
}

#[when(expr = "rowan travels via stairs at 21:30")]
fn rowan_travels_at_half_past(world: &mut World) {
    let (_mark, added) = table::player_send(world, "travel stairs", "\"kind\":");
    world.player_added = added;
}
