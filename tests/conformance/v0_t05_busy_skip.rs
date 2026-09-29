use std::fs;
use std::path::{Path, PathBuf};

use spookery_tick::{run_tick, Catchup, Reason};

use super::support::copy_dir;

fn arrange(pack: &Path) {
    let shard = spookery_kernel::open_shard(pack).unwrap_or_else(|err| panic!("open_shard: {err}"));
    let mut now = shard.now().clone();
    drop(shard);
    now.presence.humans = 1;
    now.presence.actors_present = vec!["pc_rowan".to_string()];
    now.world_time = 360;
    *now.actors
        .get_mut("mira")
        .unwrap_or_else(|| panic!("mira missing"))
        .place_mut() = "docks_shop".to_string();
    let text =
        spookery_kernel::codec::emit_now(&now).unwrap_or_else(|err| panic!("emit_now: {err}"));
    fs::write(pack.join("state/now.yaml"), text).unwrap_or_else(|err| panic!("write now: {err}"));
}

#[test]
fn v0_t05_busy_skip() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let pack = std::env::temp_dir().join(format!(
        "spook-tick05-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&pack);
    copy_dir(&manifest.join("packs/oneshot-docks"), &pack);
    arrange(&pack);

    let mut shard =
        spookery_kernel::open_shard(&pack).unwrap_or_else(|err| panic!("open_shard: {err}"));
    let wall = shard.now().last_tick_at.clone();
    run_tick(&mut shard, Reason::Manual, &wall, Catchup::none())
        .unwrap_or_else(|err| panic!("tick: {err}"));

    let now = shard.now();
    assert_eq!(
        now.world_time, 365,
        "a present player still advances world_time"
    );
    assert_eq!(now.actors.get("mira").unwrap().place(), "docks_shop");
    assert_eq!(
        now.actors
            .get("mira")
            .unwrap()
            .tick()
            .unwrap()
            .last_act_tick,
        0,
        "mira files no idle move while the player is in docks_shop"
    );
    assert_eq!(
        now.places
            .get("docks_shop")
            .and_then(|place| place.stock.get("lamp_oil"))
            .copied(),
        Some(4)
    );
    assert_eq!(
        now.actors.get("bram").unwrap().place(),
        "docks_shop",
        "an npc in another place still acts"
    );
    assert_eq!(
        now.actors
            .get("bram")
            .unwrap()
            .tick()
            .unwrap()
            .last_act_tick,
        1
    );

    let _ = fs::remove_dir_all(&pack);
}
