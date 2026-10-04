use std::fs;
use std::path::{Path, PathBuf};

use spookery_kernel::apply::apply_effect;
use spookery_kernel::codec::emit_now;
use spookery_kernel::effect::{Effect, EffectClass};
use spookery_kernel::model::{ActorKind, Intent, IntentArgs, Now, Source, Target, ViewerId};
use spookery_kernel::shard::Txn;

use super::support::copy_dir;

fn temp_pack(label: &str) -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixture = manifest.join("packs/oneshot-docks");
    let pack = std::env::temp_dir().join(format!(
        "spook-tick06-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&pack);
    copy_dir(&fixture, &pack);
    pack
}

fn lamp_oil(now: &Now) -> i64 {
    now.places
        .get("docks_shop")
        .and_then(|place| place.stock.get("lamp_oil"))
        .copied()
        .unwrap_or_else(|| panic!("lamp_oil missing"))
}

fn mira_intent() -> Intent {
    Intent {
        id: "drop-txn".to_string(),
        tick_index: 1,
        actor_id: "mira".to_string(),
        actor_kind: ActorKind::Npc,
        source: Source::Kernel,
        move_name: "stock".to_string(),
        target: Target::Place("docks_shop".to_string()),
        args: IntentArgs::default(),
        rationale_code: String::new(),
    }
}

fn stock_effect(delta: i64) -> Effect {
    Effect {
        class: EffectClass::Stock {
            place: "docks_shop".to_string(),
            sku: "lamp_oil".to_string(),
            delta,
        },
        as_viewer: ViewerId::Npc("mira".to_string()),
        intent_id: "drop-txn".to_string(),
    }
}

fn apply_stock_then_drop(pack: &Path) {
    let now_path = pack.join("state/now.yaml");
    let before = fs::read(&now_path).unwrap_or_else(|err| panic!("read now: {err}"));
    let shard = spookery_kernel::open_shard(pack).unwrap_or_else(|err| panic!("open_shard: {err}"));
    let state_rev = shard.now().state_rev;
    let proposals = shard.now().proposals.clone();
    let oil = lamp_oil(shard.now());
    let delta = if oil > 0 { -1 } else { 1 };
    let mut txn = Txn::begin(&shard);
    apply_effect(&mut txn, &mira_intent(), stock_effect(delta))
        .unwrap_or_else(|err| panic!("apply_effect: {err}"));
    assert_eq!(
        lamp_oil(txn.now()),
        oil + delta,
        "staging did not take the stock effect"
    );
    drop(txn);
    drop(shard);

    assert!(
        !pack.join("state/.txn").exists(),
        "dropping the Txn created state/.txn"
    );
    assert_eq!(
        fs::read(&now_path).unwrap_or_else(|err| panic!("reread now: {err}")),
        before,
        "now.yaml bytes changed"
    );

    let restarted =
        spookery_kernel::open_shard(pack).unwrap_or_else(|err| panic!("restart: {err}"));
    assert_eq!(restarted.now().state_rev, state_rev, "state_rev changed");
    assert_eq!(lamp_oil(restarted.now()), oil, "stock value changed");
    assert_eq!(
        restarted.now().proposals,
        proposals,
        "proposal list changed"
    );
    assert!(
        !pack.join("state/proposals").exists(),
        "a proposal file appeared"
    );
    assert!(
        !pack.join("state/memories").exists(),
        "a memory file appeared"
    );
}

fn write_now(pack: &Path, now: &Now) {
    let text = emit_now(now).unwrap_or_else(|err| panic!("emit_now: {err}"));
    fs::write(pack.join("state/now.yaml"), text).unwrap_or_else(|err| panic!("write now: {err}"));
}

fn recover_incomplete(pack: &Path) {
    let now_path = pack.join("state/now.yaml");
    let before = fs::read(&now_path).unwrap_or_else(|err| panic!("read now: {err}"));
    let txn = pack.join("state/.txn");
    fs::create_dir_all(txn.join("prev")).unwrap_or_else(|err| panic!("mkdir txn: {err}"));
    fs::write(txn.join("prev/now.yaml"), b"not-yaml\n")
        .unwrap_or_else(|err| panic!("write: {err}"));
    let opened = spookery_kernel::open_shard(pack).unwrap_or_else(|err| panic!("recover: {err}"));
    assert_eq!(
        fs::read(&now_path).unwrap_or_else(|err| panic!("reread: {err}")),
        before,
        "incomplete .txn rewrote now.yaml"
    );
    assert_eq!(lamp_oil(opened.now()), lamp_oil_of_bytes(&before));
    assert!(!txn.exists(), "incomplete .txn was kept");
}

fn recover_complete_same_rev(pack: &Path) {
    let now_path = pack.join("state/now.yaml");
    let original = fs::read(&now_path).unwrap_or_else(|err| panic!("read now: {err}"));
    let shard = spookery_kernel::open_shard(pack).unwrap_or_else(|err| panic!("open: {err}"));
    let mut dirty = shard.now().clone();
    drop(shard);
    dirty
        .places
        .get_mut("docks_shop")
        .unwrap_or_else(|| panic!("docks_shop missing"))
        .stock
        .insert("lamp_oil".to_string(), 999);
    write_now(pack, &dirty);
    let txn = pack.join("state/.txn");
    fs::create_dir_all(txn.join("prev")).unwrap_or_else(|err| panic!("mkdir txn: {err}"));
    fs::write(txn.join("prev/now.yaml"), &original)
        .unwrap_or_else(|err| panic!("write prev: {err}"));
    fs::write(txn.join("COMPLETE"), b"").unwrap_or_else(|err| panic!("write COMPLETE: {err}"));

    let opened = spookery_kernel::open_shard(pack).unwrap_or_else(|err| panic!("recover: {err}"));
    assert_eq!(
        fs::read(&now_path).unwrap_or_else(|err| panic!("reread: {err}")),
        original,
        "COMPLETE with the same state_rev did not restore prev"
    );
    assert_eq!(lamp_oil(opened.now()), lamp_oil_of_bytes(&original));
    assert!(!txn.exists(), ".txn survived restore");
}

fn recover_complete_next_rev(pack: &Path) {
    let shard = spookery_kernel::open_shard(pack).unwrap_or_else(|err| panic!("open: {err}"));
    let prev = shard.now().clone();
    let mut live = prev.clone();
    drop(shard);
    live.state_rev = prev.state_rev.saturating_add(1);
    let kept_oil = lamp_oil(&prev).saturating_add(1);
    live.places
        .get_mut("docks_shop")
        .unwrap_or_else(|| panic!("docks_shop missing"))
        .stock
        .insert("lamp_oil".to_string(), kept_oil);
    let prev_bytes = emit_now(&prev).unwrap_or_else(|err| panic!("emit prev: {err}"));
    write_now(pack, &live);

    let txn = pack.join("state/.txn");
    fs::create_dir_all(txn.join("prev")).unwrap_or_else(|err| panic!("mkdir txn: {err}"));
    fs::write(txn.join("prev/now.yaml"), prev_bytes)
        .unwrap_or_else(|err| panic!("write prev: {err}"));
    fs::write(txn.join("COMPLETE"), b"").unwrap_or_else(|err| panic!("write COMPLETE: {err}"));

    let log = pack.join("state/log");
    fs::create_dir_all(&log).unwrap_or_else(|err| panic!("mkdir log: {err}"));
    fs::write(
        log.join("tick-0001.jsonl"),
        format!(
            "{{\"tick_index\":1,\"state_rev_after\":{}}}\n",
            live.state_rev
        ),
    )
    .unwrap_or_else(|err| panic!("write kept log: {err}"));
    fs::write(
        log.join("tick-0099.jsonl"),
        format!(
            "{{\"tick_index\":99,\"state_rev_after\":{}}}\n",
            live.state_rev.saturating_add(5)
        ),
    )
    .unwrap_or_else(|err| panic!("write orphan log: {err}"));

    let memories = pack.join("state/memories");
    fs::create_dir_all(&memories).unwrap_or_else(|err| panic!("mkdir memories: {err}"));
    fs::write(
        memories.join("mira.jsonl"),
        "{\"tick_index\":1,\"text\":\"kept-line\"}\n{\"tick_index\":99,\"text\":\"orphan-line\"}\n",
    )
    .unwrap_or_else(|err| panic!("write memories: {err}"));

    let proposals = pack.join("state/proposals");
    fs::create_dir_all(&proposals).unwrap_or_else(|err| panic!("mkdir proposals: {err}"));
    fs::write(proposals.join("orphan.yaml"), b"orphan\n")
        .unwrap_or_else(|err| panic!("write proposal: {err}"));

    let opened = spookery_kernel::open_shard(pack).unwrap_or_else(|err| panic!("recover: {err}"));
    assert_eq!(
        opened.now().state_rev,
        live.state_rev,
        "live rev was restored"
    );
    assert_eq!(lamp_oil(opened.now()), kept_oil, "live stock was restored");
    assert!(!txn.exists(), ".txn survived the completed commit");
    assert!(
        log.join("tick-0001.jsonl").is_file(),
        "live tick log was deleted"
    );
    assert!(
        !log.join("tick-0099.jsonl").exists(),
        "orphan tick log was kept"
    );
    let memory = fs::read_to_string(memories.join("mira.jsonl"))
        .unwrap_or_else(|err| panic!("read memories: {err}"));
    assert!(memory.contains("kept-line"), "live memory line was dropped");
    assert!(
        !memory.contains("orphan-line"),
        "orphan memory line was kept"
    );
    assert!(
        !proposals.join("orphan.yaml").exists(),
        "orphan proposal was kept"
    );
}

fn lamp_oil_of_bytes(bytes: &[u8]) -> i64 {
    let text = std::str::from_utf8(bytes).unwrap_or_else(|err| panic!("utf8: {err}"));
    let now = spookery_kernel::codec::load_now(text).unwrap_or_else(|err| panic!("load: {err}"));
    lamp_oil(&now)
}

#[test]
fn v0_t06_atomic_commit() {
    let dropped = temp_pack("drop");
    apply_stock_then_drop(&dropped);
    let _ = fs::remove_dir_all(&dropped);

    let incomplete = temp_pack("incomplete");
    recover_incomplete(&incomplete);
    let _ = fs::remove_dir_all(&incomplete);

    let same_rev = temp_pack("same-rev");
    recover_complete_same_rev(&same_rev);
    let _ = fs::remove_dir_all(&same_rev);

    let next_rev = temp_pack("next-rev");
    recover_complete_next_rev(&next_rev);
    let _ = fs::remove_dir_all(&next_rev);
}
