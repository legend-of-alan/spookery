use spookery_engine::testing::{place_of, world_of};
use spookery_engine::{component, entity, ActorId};
use std::fs;
use std::path::{Path, PathBuf};

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
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/packs/docks");
    spookery_engine::pack_rev(&root).unwrap()
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
async fn one_round_has_committed(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::run(world, &["round", pack.to_str().unwrap()]).await;
    assert_eq!(world.code, Some(0), "{}", world.stderr);
    assert!(
        !spookery_engine::testing::journal::list_round_keys(&pack).is_empty(),
        "no round committed"
    );
}

#[when(expr = "the operator tries a round")]
async fn the_operator_tries_a_round(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    world.state_files = read_state_files(&pack);
    table::run(world, &["round", pack.to_str().unwrap()]).await;
}

#[when(expr = "the operator runs migrate")]
async fn the_operator_runs_migrate(world: &mut World) {
    // repeats module: mirrors its step text
    let pack = table::pack(world).to_path_buf();
    table::run(world, &["migrate", pack.to_str().unwrap()]).await;
}

#[when(expr = "the operator migrates twice")]
async fn the_operator_migrates_twice(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    let arg = pack.to_str().unwrap();
    for _ in 0..2 {
        table::run(world, &["migrate", arg]).await;
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

#[then(expr = "every actor reads {string} at {int} with no row in the save")]
fn every_actor_reads_meter_at(world: &mut World, id: String, value: i64) {
    let pack = table::pack(world);
    let (now, saved) = (world_of(pack), support::save_of(pack));
    for actor in now.actors().query::<ActorId>().iter() {
        let carrier = entity::Id::Actor(actor.clone());
        let meter = now.entities().get::<component::Meter>(&carrier);
        assert_eq!(meter.value(&id), Ok(value), "{actor} {id}");
        let row = &saved["actors"]["meters"][actor.as_str()][&id];
        assert!(row.is_null(), "{actor} {id}: {row}");
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

#[then(expr = "spook check reports {word}")]
async fn spook_check_reports(world: &mut World, code: String) {
    let pack = table::pack(world).to_path_buf();
    table::run(world, &["check", pack.to_str().unwrap()]).await;
    assert_eq!(world.code, Some(1), "{}\n{}", world.stdout, world.stderr);
    assert!(world.stderr.contains(&code), "{}", world.stderr);
}

#[then(expr = "the host refuses to start with {word}")]
async fn the_host_refuses_to_start(world: &mut World, code: String) {
    let pack = table::pack(world).to_path_buf();
    let arg = support::pack_arg(&pack);
    let mut proc = support::spawn(&["run", &arg, "--addr", "127.0.0.1:0"]);
    let (status, _, stderr) = proc.wait_with_output().await;
    world.code = status.code();
    world.stderr = stderr;
    assert_eq!(world.code, Some(1), "{}", world.stderr);
    assert!(world.stderr.contains(&code), "{}", world.stderr);
}

#[then(expr = "state_rev is 2")]
fn state_rev_is_2(world: &mut World) {
    let now = world_of(table::pack(world));
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

#[then(expr = "migrate keeps the hand edit")]
fn migrate_keeps_the_hand_edit(world: &mut World) {
    // repeats module: mirrors its step text
    let pack = table::pack(world);
    let rev = spookery_engine::pack_rev(pack).unwrap();
    assert_ne!(rev, docks_pack_rev());
    let now = world_of(pack);
    assert_eq!(now.pack_rev, rev);
    assert_eq!(now.state_rev, 1);
    assert_eq!(now.world_time, 7);
    assert_eq!(now.round_index, 0);
    assert_eq!(now.last_round_at, "2026-01-01T00:00:00Z");
    assert_eq!(place_of(&now, "rowan"), "mira_rooms");
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
    let now = world_of(table::pack(world));
    let shop = "docks_shop".into();
    let stock = now.places().get::<component::Item>(&shop);
    assert_eq!(stock.get("lamp_oil").copied(), Some(count));
}

#[then(expr = "verify trusts the operator edit at state_rev {int}")]
async fn verify_trusts_the_operator_edit(world: &mut World, rev: u64) {
    let pack = table::pack(world).to_path_buf();
    table::run(world, &["verify", pack.to_str().unwrap()]).await;
    assert_eq!(world.code, Some(0), "{}", world.stderr);
    let line = format!("operator edit trusted at state_rev {rev}");
    assert!(world.stdout.contains(&line), "{}", world.stdout);
}

#[when(
    expr = "Rowan's vigor offset is hand edited to {int} and Nell joins with no meter or act rows"
)]
fn rowan_moves_vigor_and_nell_joins(world: &mut World, offset: i64) {
    let pack = table::pack(world).to_path_buf();
    let nell = "---\nschema: cast\nid: nell\ncontrol_type: pc_template\nname: Nell\n---\nNell.\n";
    fs::write(pack.join("cast/pc-nell.md"), nell).unwrap_or_else(|err| panic!("nell: {err}"));
    spookery_engine::testing::edit_save(&pack, |now| {
        support::edit_meter(now, "rowan", "vigor", |vigor| vigor.offset = offset);
        let nell = "nell".into();
        let mut actors = now.actors_mut();
        actors.add_actor(&nell, component::ControlType::Pc, "docks_shop");
    });
}

#[then(expr = "{word}'s vigor is at base {int} and offset {int}")]
fn vigor_is_at(world: &mut World, actor: String, base: i64, offset: i64) {
    let now = world_of(table::pack(world));
    let id = actor.as_str().into();
    let vigor = now.actors().get::<component::Meter>(&id).get("vigor");
    let got = vigor.map(|vigor| (vigor.base, vigor.offset));
    assert_eq!(got, Some((base, offset)), "{actor}");
}
