use spookery_engine::testing::{place_of, state_of};
use spookery_engine::{component, ActorId, Meter};
use std::fs;
use std::path::{Path, PathBuf};

use cucumber::gherkin::Step;
use cucumber::{given, then, when};

use crate::support;
use crate::World;

use super::table;

fn read_state_files(pack: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let state = pack.join("state");
    if state.is_dir() {
        table::read_files(&state)
    } else {
        Vec::new()
    }
}

fn docks_pack_rev() -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packs/oneshot-docks");
    spookery_engine::pack_rev(&root).unwrap()
}

fn read_migrate_revs(pack: &Path) -> Vec<u64> {
    use spookery_engine::testing::journal::{read_records, Record};
    let entries = read_records(pack).unwrap_or_else(|err| panic!("{}", err.detail));
    let mut revs = Vec::new();
    for entry in &entries {
        let Record::Migrate(record) = entry else {
            continue;
        };
        revs.push(record.state_rev_after);
    }
    revs
}

#[when(expr = "a canon file gains a byte")]
fn a_canon_file_gains_a_byte(world: &mut World) {
    let pack = table::pack(world);
    let mira = pack.join("cast/mira.md");
    let mut body = fs::read_to_string(&mira).unwrap();
    body.push('\n');
    fs::write(&mira, body).unwrap();
    world.now_bytes = fs::read(pack.join("state/save.yaml")).unwrap();
}

#[given(expr = "one round has committed")]
fn one_round_has_committed(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::run(world, &["round", pack.to_str().unwrap()]);
    assert_eq!(world.code, Some(0), "{}", world.stderr);
    assert!(
        !spookery_engine::testing::journal::list_round_keys(&pack).is_empty(),
        "no round committed"
    );
}

#[when(expr = "the operator tries a round")]
fn the_operator_tries_a_round(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    world.state_files = read_state_files(&pack);
    table::run(world, &["round", pack.to_str().unwrap()]);
}

#[when(expr = "the operator runs migrate")]
fn the_operator_runs_migrate(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::run(world, &["migrate", pack.to_str().unwrap()]);
}

#[when(expr = "the operator migrates twice")]
fn the_operator_migrates_twice(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    let arg = pack.to_str().unwrap();
    for _ in 0..2 {
        table::run(world, &["migrate", arg]);
        assert_eq!(world.code, Some(0), "{}", world.stderr);
        assert!(world.stdout.is_empty(), "{}", world.stdout);
        assert!(world.stderr.is_empty(), "{}", world.stderr);
    }
}

#[when(expr = "the save is hand edited and the canon gains a byte")]
fn the_save_is_hand_edited(world: &mut World) {
    let pack = table::pack(world);
    let now_path = pack.join("state/save.yaml");
    let mut text = fs::read_to_string(&now_path).unwrap();
    text = text.replace("world_time: 1080", "world_time: 7");
    text = text.replace("    rowan: docks_shop\n", "    rowan: mira_rooms\n");
    fs::write(&now_path, text).unwrap();
    a_canon_file_gains_a_byte(world);
}

#[when(expr = "the pack adds meter {string} with start {int}")]
fn the_pack_adds_meter(world: &mut World, id: String, start: i64) {
    let path = table::pack(world).join("rules/meters.yaml");
    let mut text = fs::read_to_string(&path).unwrap();
    text.push_str(&format!(
        "  - id: {id}\n    min: 0\n    base_cap: 10\n    max: 10\n    base: {start}\n"
    ));
    fs::write(&path, text).unwrap();
}

#[then(expr = "every actor has {string} at {int}")]
fn every_actor_has_meter_at(world: &mut World, id: String, base: i64) {
    let now = state_of(table::pack(world));
    for actor in now.actors().query::<ActorId>().iter() {
        let meter = now.actors().get::<component::Meter>(actor).get(&id);
        let got = meter.map(|meter| meter.base);
        assert_eq!(got, Some(base), "{actor} {id}");
    }
}

#[when(expr = "the save is replaced with a broken document")]
fn the_save_is_replaced(world: &mut World) {
    let pack = table::pack(world);
    fs::write(pack.join("state/save.yaml"), "schema: no\n").unwrap();
    world.before = table::read_files(pack);
}

#[then(expr = "stderr names {word}")]
fn stderr_names(world: &mut World, code: String) {
    assert!(world.stderr.contains(&code), "{}", world.stderr);
}

#[then(expr = "the round writes nothing")]
fn the_round_writes_nothing(world: &mut World) {
    let pack = table::pack(world);
    assert_eq!(
        fs::read(pack.join("state/save.yaml")).unwrap(),
        world.now_bytes
    );
    assert_eq!(
        read_state_files(pack),
        world.state_files,
        "the round wrote under state/"
    );
}

#[when(expr = "the save drops meter {string}")]
fn the_save_drops_meter(world: &mut World, id: String) {
    let path = table::pack(world).join("state/save.yaml");
    let text = fs::read_to_string(&path).unwrap();
    let needle = format!("      {id}:");
    let mut out = String::new();
    let mut skip = 0;
    let mut found = false;
    for line in text.lines() {
        if skip > 0 {
            skip -= 1;
            continue;
        }
        if line == needle {
            found = true;
            skip = 2;
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    assert!(found, "save has no meter {id}");
    fs::write(&path, out).unwrap();
}

#[then(expr = "spook check reports {word}")]
fn spook_check_reports(world: &mut World, code: String) {
    let pack = table::pack(world).to_path_buf();
    table::run(world, &["check", pack.to_str().unwrap()]);
    assert_eq!(world.code, Some(1), "{}\n{}", world.stdout, world.stderr);
    assert!(world.stderr.contains(&code), "{}", world.stderr);
}

#[then(expr = "the host refuses to start with {word}")]
fn the_host_refuses_to_start(world: &mut World, code: String) {
    let pack = table::pack(world).to_path_buf();
    let arg = support::pack_arg(&pack);
    let mut proc = support::spawn(&["run", &arg, "--addr", "127.0.0.1:0"]);
    let (status, _, stderr) = proc.wait_with_output();
    world.code = status.code();
    world.stderr = stderr;
    assert_eq!(world.code, Some(1), "{}", world.stderr);
    assert!(world.stderr.contains(&code), "{}", world.stderr);
}

#[then(expr = "state_rev is 2")]
fn state_rev_is_2(world: &mut World) {
    let now = state_of(table::pack(world));
    assert_eq!(now.state_rev, 2);
    assert_eq!(now.pack_rev, docks_pack_rev());
    assert_eq!(place_of(&now, "rowan"), "docks_shop");
    assert_eq!(
        now.places()
            .get::<component::Item>(&"docks_shop".into())
            .get("lamp_oil")
            .copied(),
        Some(4u32)
    );
    assert_eq!(now.last_round_at, "2026-01-01T00:00:00Z");
}

#[then(expr = "the journal has two migrate records, at state_rev 1 and 2")]
fn the_journal_has_two_migrate_records(world: &mut World) {
    assert_eq!(read_migrate_revs(table::pack(world)), [1, 2]);
}

#[then(expr = "migrate keeps the hand edit")]
fn migrate_keeps_the_hand_edit(world: &mut World) {
    let pack = table::pack(world);
    let rev = spookery_engine::pack_rev(pack).unwrap();
    assert_ne!(rev, docks_pack_rev());
    let now = state_of(pack);
    assert_eq!(now.pack_rev, rev);
    assert_eq!(now.state_rev, 1);
    assert_eq!(now.world_time, 7);
    assert_eq!(now.round_index, 0);
    assert_eq!(now.last_round_at, "2026-01-01T00:00:00Z");
    assert_eq!(place_of(&now, "rowan"), "mira_rooms");
    assert!(spookery_engine::testing::journal::list_round_keys(pack).is_empty());
    assert_eq!(read_migrate_revs(pack), [1]);
}

#[when(expr = "the shop's lamp oil is hand edited to {int}")]
fn the_shops_lamp_oil_is_hand_edited(world: &mut World, count: u32) {
    let pack = table::pack(world);
    let path = pack.join("state/save.yaml");
    let text = fs::read_to_string(&path).unwrap();
    let edited = text.replacen(
        "      lamp_oil: 4\n",
        &format!("      lamp_oil: {count}\n"),
        1,
    );
    assert_ne!(edited, text, "the shop did not hold 4 lamp oil\n{text}");
    fs::write(&path, edited).unwrap();
    world.before = table::read_files(pack);
}

#[then(expr = "the shop has {int} lamp oil")]
fn the_shop_has_lamp_oil(world: &mut World, count: u32) {
    let now = state_of(table::pack(world));
    let shop = "docks_shop".into();
    let stock = now.places().get::<component::Item>(&shop);
    assert_eq!(stock.get("lamp_oil").copied(), Some(count));
}

#[then(expr = "verify trusts the operator edit at state_rev {int}")]
fn verify_trusts_the_operator_edit(world: &mut World, rev: u64) {
    let pack = table::pack(world).to_path_buf();
    table::run(world, &["verify", pack.to_str().unwrap()]);
    assert_eq!(world.code, Some(0), "{}", world.stderr);
    let line = format!("operator edit trusted at state_rev {rev}");
    assert!(world.stdout.contains(&line), "{}", world.stdout);
}

#[then(expr = "the journal line {word} is")]
fn the_journal_line_is(world: &mut World, key: String, step: &Step) {
    let line = spookery_engine::testing::journal::read_record_line(table::pack(world), &key)
        .unwrap_or_else(|err| panic!("{}", err.detail));
    let golden = step.docstring.as_deref().unwrap_or_default();
    assert_eq!(line, golden.trim());
}

#[when(expr = "Rowan's grit offset is hand edited to {int} and Nell joins without grit")]
fn rowan_moves_grit_and_nell_joins(world: &mut World, offset: i64) {
    let pack = table::pack(world).to_path_buf();
    spookery_engine::testing::edit_save(&pack, |now| {
        support::edit_meter(now, "rowan", "grit", |grit| grit.offset = offset);
        let (rowan, nell) = (&"rowan".into(), &"nell".into());
        let mut actors = now.actors_mut();
        actors.copy_actor(rowan, nell, "docks_shop").unwrap();
        actors.get_mut::<Meter>(nell).set("grit", None).unwrap();
    });
}

#[then(expr = "{word}'s grit is at base {int} and offset {int}")]
fn grit_is_at(world: &mut World, actor: String, base: i64, offset: i64) {
    let now = state_of(table::pack(world));
    let id = actor.as_str().into();
    let grit = now.actors().get::<component::Meter>(&id).get("grit");
    let got = grit.map(|grit| (grit.base, grit.offset));
    assert_eq!(got, Some((base, offset)), "{actor}");
}
