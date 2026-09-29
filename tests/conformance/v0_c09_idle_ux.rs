use std::fs;
use std::path::{Path, PathBuf};

use spookery_kernel::perceive::StockView;
use spookery_kernel::DigestViewer;

use super::support::{copy_dir, spook};

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

#[test]
fn v0_c09_idle_ux() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixture = manifest.join("packs/oneshot-docks");
    let pack = std::env::temp_dir().join(format!(
        "spook-product09-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&pack);
    copy_dir(&fixture, &pack);
    let pack_arg = pack.to_str().unwrap();

    for tick in 1..=13 {
        let output = spook(&["tick", pack_arg]);
        assert!(
            output.status.success(),
            "tick {tick}\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let before = files(&pack);
    let status = spook(&["status", pack_arg]);
    assert!(
        status.status.success(),
        "status\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&status.stdout),
        String::from_utf8_lossy(&status.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&status.stdout), HOST);
    assert_eq!(files(&pack), before, "status wrote the pack");

    let shard =
        spookery_kernel::open_shard(&pack).unwrap_or_else(|err| panic!("open_shard: {err}"));
    let now = shard.now();
    assert_eq!(now.tick_index, 13);
    assert_eq!(now.world_time, 1860);
    assert_eq!(now.last_tick_at, "2026-01-01T00:00:00Z");
    assert_eq!(
        now.places
            .get("docks_shop")
            .and_then(|place| place.stock.get("lamp_oil"))
            .copied(),
        Some(8)
    );
    let debt = now
        .clocks
        .get("assassination_debt")
        .unwrap_or_else(|| panic!("assassination_debt missing"));
    assert_eq!(debt.value, 2);
    assert!(debt.paused);

    let resume = spookery_kernel::idle_digest(&shard, DigestViewer::Player("pc_rowan".to_string()))
        .unwrap_or_else(|err| panic!("resume: {err}"));
    assert_eq!(resume, RESUME);
    assert!(!resume.contains("lamp_oil"));

    let shop = spookery_kernel::perceive(&shard, "player:pc_rowan")
        .unwrap_or_else(|err| panic!("perceive shop: {err}"));
    assert_eq!(shop.place, "docks_shop");
    assert_eq!(
        shop.stocks,
        vec![StockView {
            sku: "lamp_oil".to_string(),
            qty: 8,
        }]
    );

    let mut away = now.clone();
    *away
        .actors
        .get_mut("pc_rowan")
        .unwrap_or_else(|| panic!("pc_rowan missing"))
        .place_mut() = "mira_rooms".to_string();
    let rooms = spookery_kernel::perceive::perceive_at(shard.canon(), &away, "player:pc_rowan")
        .unwrap_or_else(|err| panic!("perceive rooms: {err}"));
    assert_eq!(rooms.place, "mira_rooms");
    assert!(rooms.stocks.iter().all(|stock| stock.sku != "lamp_oil"));

    let _ = fs::remove_dir_all(&pack);
}
