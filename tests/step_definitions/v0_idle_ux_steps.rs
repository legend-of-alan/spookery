use std::fs;
use std::path::{Path, PathBuf};

use cucumber::{then, when};

use spookery_kernel::perceive::StockView;
use spookery_kernel::DigestViewer;

use crate::support::spook;
use crate::World;

const HOST: &str = "\
effect message 1 {\"message_id\":\"note_mira_door\",\"to\":[\"pc_rowan\"],\"place\":\"docks_shop\",\"fact_ids\":[],\"text\":\"Back at dusk. Oil on the shelf.\"}
effect location 12 {\"actor_id\":\"bram\",\"from\":\"docks_street\",\"to\":\"docks_shop\"}
effect location 12 {\"actor_id\":\"mira\",\"from\":\"mira_rooms\",\"to\":\"docks_shop\"}
effect stock 13 {\"place\":\"docks_shop\",\"sku\":\"lamp_oil\",\"delta\":4}
effect rumor 13 {\"rumor_id\":\"r_184\",\"fact_ids\":[\"fact_oil_on_shelf\"],\"from\":\"bram\",\"to\":\"aldric\",\"heat\":1}
clock assassination_debt 2 6 paused
clock shop_day 1 1 running
";

const RESUME: &str = "\
message note_mira_door Back at dusk. Oil on the shelf.
clock assassination_debt 2 6 paused
";

fn files(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries =
            fs::read_dir(&dir).unwrap_or_else(|err| panic!("read {}: {err}", dir.display()));
        for entry in entries {
            let entry = entry.unwrap_or_else(|err| panic!("dirent {}: {err}", dir.display()));
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let bytes =
                    fs::read(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
                out.push((path, bytes));
            }
        }
    }
    out.sort();
    out
}

#[then(expr = "clock {word} value is {int}")]
fn clock_value(w: &mut World, name: String, value: i64) {
    let shard = spookery_kernel::open_shard(w.pack.as_ref().unwrap())
        .unwrap_or_else(|err| panic!("open_shard: {err}"));
    let clock = shard
        .now()
        .clocks
        .get(&name)
        .unwrap_or_else(|| panic!("{name} missing"));
    assert_eq!(clock.value, value);
}

#[when(expr = "spook status is run")]
fn run_status(w: &mut World) {
    let pack = w.pack.as_ref().unwrap().clone();
    let before = files(&pack);
    let output = spook(&["status", pack.to_str().unwrap()]);
    let after = files(&pack);
    w.scratch.insert(
        "idle-status-ok".to_string(),
        output.status.success().to_string(),
    );
    w.scratch.insert(
        "idle-status-stdout".to_string(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
    );
    w.scratch.insert(
        "idle-status-stderr".to_string(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    );
    w.scratch.insert(
        "idle-status-pack-unchanged".to_string(),
        (before == after).to_string(),
    );
}

#[then(expr = "spook status exits 0")]
fn status_exits_0(w: &mut World) {
    assert_eq!(
        w.scratch.get("idle-status-ok").map(String::as_str),
        Some("true"),
        "status\nstdout:\n{}\nstderr:\n{}",
        w.scratch.get("idle-status-stdout").map(String::as_str).unwrap_or(""),
        w.scratch.get("idle-status-stderr").map(String::as_str).unwrap_or("")
    );
}

#[then(expr = "spook status stdout is the host digest")]
fn status_stdout(w: &mut World) {
    assert_eq!(
        w.scratch.get("idle-status-stdout").map(String::as_str),
        Some(HOST)
    );
}

#[then(expr = "status leaves the pack bytes unchanged")]
fn status_leaves_pack(w: &mut World) {
    assert_eq!(
        w.scratch.get("idle-status-pack-unchanged").map(String::as_str),
        Some("true"),
        "status wrote the pack"
    );
}

#[then(expr = "the player resume for pc_rowan is the idle digest")]
fn resume_is(w: &mut World) {
    let shard = spookery_kernel::open_shard(w.pack.as_ref().unwrap())
        .unwrap_or_else(|err| panic!("open_shard: {err}"));
    let resume = spookery_kernel::idle_digest(&shard, DigestViewer::Player("pc_rowan".to_string()))
        .unwrap_or_else(|err| panic!("resume: {err}"));
    assert_eq!(resume, RESUME);
}

#[then(expr = "the player resume for pc_rowan does not contain lamp_oil")]
fn resume_hides_stock(w: &mut World) {
    let shard = spookery_kernel::open_shard(w.pack.as_ref().unwrap())
        .unwrap_or_else(|err| panic!("open_shard: {err}"));
    let resume = spookery_kernel::idle_digest(&shard, DigestViewer::Player("pc_rowan".to_string()))
        .unwrap_or_else(|err| panic!("resume: {err}"));
    assert!(!resume.contains("lamp_oil"));
}

#[then(expr = "perceive as player:pc_rowan shows docks_shop stock lamp_oil {int}")]
fn perceive_shop(w: &mut World, qty: i64) {
    let shard = spookery_kernel::open_shard(w.pack.as_ref().unwrap())
        .unwrap_or_else(|err| panic!("open_shard: {err}"));
    let shop = spookery_kernel::perceive(&shard, "player:pc_rowan")
        .unwrap_or_else(|err| panic!("perceive shop: {err}"));
    assert_eq!(shop.place, "docks_shop");
    assert_eq!(
        shop.stocks,
        vec![StockView {
            sku: "lamp_oil".to_string(),
            qty,
        }]
    );
}

#[then(expr = "perceive as player:pc_rowan from mira_rooms shows no lamp_oil")]
fn perceive_rooms(w: &mut World) {
    let shard = spookery_kernel::open_shard(w.pack.as_ref().unwrap())
        .unwrap_or_else(|err| panic!("open_shard: {err}"));
    let mut away = shard.now().clone();
    *away
        .actors
        .get_mut("pc_rowan")
        .unwrap_or_else(|| panic!("pc_rowan missing"))
        .place_mut() = "mira_rooms".to_string();
    let rooms = spookery_kernel::perceive::perceive_at(shard.canon(), &away, "player:pc_rowan")
        .unwrap_or_else(|err| panic!("perceive rooms: {err}"));
    assert_eq!(rooms.place, "mira_rooms");
    assert!(rooms.stocks.iter().all(|stock| stock.sku != "lamp_oil"));
}
