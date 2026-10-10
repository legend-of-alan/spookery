use std::fs;

use cucumber::{then, when};

use crate::support::spook;
use crate::World;

use super::table;

#[when(expr = "the operator runs spook check")]
fn the_operator_runs_spook_check(world: &mut World) {
    let pack = table::pack(world);
    let output = spook(&["check", &crate::support::pack_arg(pack)]);
    world.code = output.status.code();
    world.stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    world.stderr = String::from_utf8_lossy(&output.stderr).into_owned();
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

#[then(expr = "the report names {string}")]
fn the_report_names(world: &mut World, text: String) {
    assert!(
        world.stderr.contains(&text),
        "missing {text}\n{}",
        world.stderr
    );
}

#[then(expr = "the report says {string}")]
fn the_report_says(world: &mut World, text: String) {
    assert!(
        world.stderr.contains(&text),
        "missing {text}\n{}",
        world.stderr
    );
}

#[then(expr = "the report has a location for {string}")]
fn the_report_has_a_location_for(world: &mut World, file: String) {
    let located = world.stderr.lines().any(|line| {
        let Some(rest) = line.strip_prefix(&file) else {
            return false;
        };
        rest.starts_with(':')
            && line.contains('(')
            && (rest.contains(": ")
                || rest[1..]
                    .chars()
                    .next()
                    .is_some_and(|ch| ch.is_ascii_digit()))
    });
    assert!(located, "no location for {file}\n{}", world.stderr);
}

fn break_pack(pack: &std::path::Path, fault: &str) {
    match fault {
        "no audience" => replace_file(
            &pack.join("rules/moves.yaml"),
            "  - name: grit\n    audience: player\n",
            "  - name: grit\n",
        ),
        "unknown field" => replace_file(
            &pack.join("rules/moves.yaml"),
            "  - name: grit\n    audience: player\n",
            "  - name: grit\n    audiance: player\n",
        ),
        "unknown check" => replace_file(
            &pack.join("rules/moves.yaml"),
            "    check: grit\n",
            "    check: courage\n",
        ),
        "duplicate move" => replace_file(
            &pack.join("rules/moves.yaml"),
            "  - name: wait\n",
            "  - name: grit\n",
        ),
        "old format tag" => replace_file(
            &pack.join("rules/moves.yaml"),
            "schema: moves\n",
            "schema: moves/v0\n",
        ),
        "missing meters" => {
            fs::remove_file(pack.join("rules/meters.yaml")).unwrap();
        }
        "pack_rev mismatch" => {
            let path = pack.join("state/now.yaml");
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
        "unknown place in a range" => replace_file(
            &pack.join("cast/mira.md"),
            "    - docks_street\n",
            "    - nowhere\n",
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

fn replace_file(path: &std::path::Path, from: &str, to: &str) {
    let text = fs::read_to_string(path).unwrap();
    assert!(text.contains(from), "missing {from} in {}", path.display());
    fs::write(path, text.replacen(from, to, 1)).unwrap();
}
