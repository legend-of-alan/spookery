use std::fs;
use std::path::Path;

use cucumber::{given, then, when};
use spookery_kernel::apply::apply_effect;
use spookery_kernel::effect::{Effect, EffectClass};
use spookery_kernel::model::{ActorKind, Intent, IntentArgs, Source, Target, ViewerId};
use spookery_kernel::shard::Txn;

use crate::World;

fn stock(shard: &spookery_kernel::shard::Shard) -> i64 {
    shard
        .now()
        .places
        .get("docks_shop")
        .and_then(|place| place.stock.get("lamp_oil"))
        .copied()
        .unwrap_or_else(|| panic!("lamp_oil missing"))
}

#[given(expr = "the shard state_rev, lamp_oil, and proposals are recorded")]
fn record_before(w: &mut World) {
    let shard = spookery_kernel::open_shard(w.pack.as_ref().unwrap())
        .unwrap_or_else(|err| panic!("open_shard: {err}"));
    w.scratch
        .insert("state_rev".to_string(), shard.now().state_rev.to_string());
    w.scratch
        .insert("lamp_oil".to_string(), stock(&shard).to_string());
    w.scratch.insert(
        "proposals".to_string(),
        format!("{:?}", shard.now().proposals),
    );
}

#[when(expr = "the pack is ticked with a crash after apply_effect")]
fn tick_crash_after_apply_effect(w: &mut World) {
    let pack = w.pack.as_ref().unwrap().clone();
    drop_stock_txn(&pack);
    w.scratch
        .insert("dropped_before_commit".to_string(), "1".to_string());
}

#[then(expr = "the tick is killed before commit")]
fn tick_killed(w: &mut World) {
    assert_eq!(
        w.scratch.get("dropped_before_commit").map(String::as_str),
        Some("1"),
        "the Txn was not dropped before commit"
    );
    assert!(
        !w.pack.as_ref().unwrap().join("state/log").exists(),
        "a tick log was committed"
    );
}

fn drop_stock_txn(pack: &Path) {
    let now_path = pack.join("state/now.yaml");
    let before = fs::read(&now_path).unwrap_or_else(|err| panic!("read now: {err}"));
    let shard = spookery_kernel::open_shard(pack).unwrap_or_else(|err| panic!("open_shard: {err}"));
    let oil = stock(&shard);
    let delta = if oil > 0 { -1 } else { 1 };
    let mut txn = Txn::begin(&shard);
    apply_effect(&mut txn, &mira_intent(), stock_effect(delta))
        .unwrap_or_else(|err| panic!("apply_effect: {err}"));
    let staged = txn
        .now()
        .places
        .get("docks_shop")
        .and_then(|place| place.stock.get("lamp_oil"))
        .copied()
        .unwrap_or_else(|| panic!("staged lamp_oil missing"));
    assert_eq!(staged, oil + delta, "staging did not take the stock effect");
    drop(txn);
    drop(shard);
    assert_eq!(
        fs::read(&now_path).unwrap_or_else(|err| panic!("reread now: {err}")),
        before,
        "now.yaml bytes changed"
    );
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

#[then(expr = "state\\/.txn does not exist")]
fn no_txn(w: &mut World) {
    assert!(
        !w.pack.as_ref().unwrap().join("state/.txn").exists(),
        "apply_effect created state/.txn"
    );
}

#[then(expr = "restart state_rev is unchanged")]
fn state_rev_unchanged(w: &mut World) {
    let shard = spookery_kernel::open_shard(w.pack.as_ref().unwrap())
        .unwrap_or_else(|err| panic!("restart: {err}"));
    let recorded: u64 = w.scratch.get("state_rev").unwrap().parse().unwrap();
    assert_eq!(shard.now().state_rev, recorded, "state_rev changed");
}

#[then(expr = "restart lamp_oil is unchanged")]
fn lamp_oil_unchanged(w: &mut World) {
    let shard = spookery_kernel::open_shard(w.pack.as_ref().unwrap())
        .unwrap_or_else(|err| panic!("restart: {err}"));
    let recorded: i64 = w.scratch.get("lamp_oil").unwrap().parse().unwrap();
    assert_eq!(stock(&shard), recorded, "stock value changed");
}

#[then(expr = "restart proposals are unchanged")]
fn proposals_unchanged(w: &mut World) {
    let shard = spookery_kernel::open_shard(w.pack.as_ref().unwrap())
        .unwrap_or_else(|err| panic!("restart: {err}"));
    let recorded = w.scratch.get("proposals").unwrap();
    assert_eq!(
        format!("{:?}", shard.now().proposals),
        *recorded,
        "proposal list changed"
    );
}

#[then(expr = "state\\/proposals does not exist")]
fn no_proposal_file(w: &mut World) {
    assert!(
        !w.pack.as_ref().unwrap().join("state/proposals").exists(),
        "a proposal file appeared"
    );
}

#[then(expr = "state\\/memories does not exist")]
fn no_memory_file(w: &mut World) {
    assert!(
        !w.pack.as_ref().unwrap().join("state/memories").exists(),
        "a memory file appeared"
    );
}
