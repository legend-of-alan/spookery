use std::path::Path;

use cucumber::{then, when};

use crate::support::spook;
use crate::World;

fn lamp_oil(pack: &Path) -> i64 {
    let shard = spookery_kernel::open_shard(pack).unwrap_or_else(|err| panic!("open_shard: {err}"));
    shard
        .now()
        .places
        .get("docks_shop")
        .and_then(|place| place.stock.get("lamp_oil"))
        .copied()
        .unwrap_or_else(|| panic!("lamp_oil missing"))
}

fn stock_rate(pack: &Path) -> i64 {
    let shard = spookery_kernel::open_shard(pack).unwrap_or_else(|err| panic!("open_shard: {err}"));
    shard
        .canon()
        .profile("mira")
        .and_then(|profile| profile.stock_rate.get("docks_shop"))
        .and_then(|rates| rates.get("lamp_oil"))
        .copied()
        .unwrap_or_else(|| panic!("stock_rate for lamp_oil missing"))
}

fn lamp_oil_series(w: &World) -> Vec<i64> {
    w.scratch
        .get("lamp_oil_series")
        .unwrap()
        .split(',')
        .map(|qty| qty.parse().unwrap())
        .collect()
}

#[then(expr = "docks_shop lamp_oil stock_rate is {int}")]
fn stock_rate_is(w: &mut World, expected: i64) {
    let rate = stock_rate(w.pack.as_ref().unwrap());
    assert_eq!(rate, expected, "docks lamp_oil stock_rate");
}

#[then(expr = "docks_shop lamp_oil is at least {int}")]
fn lamp_oil_at_least(w: &mut World, min: i64) {
    let qty = lamp_oil(w.pack.as_ref().unwrap());
    assert!(qty >= min, "start stock {qty}");
}

#[when(expr = "{int} ticks record docks_shop lamp_oil")]
fn tick_record(w: &mut World, n: usize) {
    let pack = w.pack.as_ref().unwrap().clone();
    let mut series = vec![lamp_oil(&pack)];
    for _ in 0..n {
        w.outputs.push(spook(&["tick", pack.to_str().unwrap()]));
        series.push(lamp_oil(&pack));
    }
    w.scratch.insert(
        "lamp_oil_series".into(),
        series
            .iter()
            .map(|qty| qty.to_string())
            .collect::<Vec<_>>()
            .join(","),
    );
}

#[then(expr = "each docks_shop lamp_oil change stays within stock_rate")]
fn changes_within_rate(w: &mut World) {
    let rate = stock_rate(w.pack.as_ref().unwrap());
    let series = lamp_oil_series(w);
    for (index, window) in series.windows(2).enumerate() {
        let previous = window[0];
        let qty = window[1];
        let tick = index + 1;
        let delta = qty - previous;
        assert!(
            delta.abs() <= rate,
            "tick {tick}: lamp_oil {previous} -> {qty} exceeds stock_rate {rate}"
        );
    }
}

#[then(expr = "each recorded docks_shop lamp_oil quantity is at least {int}")]
fn recorded_at_least(w: &mut World, min: i64) {
    let series = lamp_oil_series(w);
    for (index, qty) in series.iter().skip(1).enumerate() {
        let tick = index + 1;
        assert!(*qty >= min, "tick {tick}: lamp_oil {qty}");
    }
}
