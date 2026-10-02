use std::fs;
use std::path::PathBuf;

use cucumber::{given, then};

use crate::World;

#[given(expr = "the source pack packs\\/oneshot-docks")]
fn source_docks(w: &mut World) {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    w.pack = Some(manifest.join("packs/oneshot-docks"));
}

#[then(expr = "pack_rev is {string}")]
fn pack_rev_is(w: &mut World, expected: String) {
    let root = w.pack.as_ref().unwrap();
    let rev =
        spookery_kernel::canon::pack_rev(root).unwrap_or_else(|err| panic!("pack_rev: {err}"));
    assert_eq!(rev, expected);
}

#[then(expr = "{string} is {int} bytes")]
fn file_len(w: &mut World, rel: String, n: i64) {
    let bytes = read_pack_file(w, &rel);
    assert_eq!(bytes.len() as i64, n);
}

#[then(expr = "{string} blake2s is {string}")]
fn file_blake2s(w: &mut World, rel: String, expected: String) {
    let bytes = read_pack_file(w, &rel);
    assert_eq!(spookery_kernel::blake2s_hex(&bytes), expected);
}

#[then(expr = "{string} round-trips through emit_now")]
fn now_round_trip(w: &mut World, rel: String) {
    let now_bytes = read_pack_file(w, &rel);
    let now_text = std::str::from_utf8(&now_bytes).unwrap_or_else(|err| panic!("now utf-8: {err}"));
    let now =
        spookery_kernel::codec::load_now(now_text).unwrap_or_else(|err| panic!("load now: {err}"));
    let emitted =
        spookery_kernel::codec::emit_now(&now).unwrap_or_else(|err| panic!("emit now: {err}"));
    assert_eq!(emitted.as_bytes(), now_bytes.as_slice());
}

#[then(expr = "now pack_rev is {string}")]
fn now_pack_rev_is(w: &mut World, expected: String) {
    let now_bytes = read_pack_file(w, "state/now.yaml");
    let now_text = std::str::from_utf8(&now_bytes).unwrap_or_else(|err| panic!("now utf-8: {err}"));
    let now =
        spookery_kernel::codec::load_now(now_text).unwrap_or_else(|err| panic!("load now: {err}"));
    assert_eq!(now.pack_rev, expected);
}

#[then(expr = "{string} round-trips through sheet emit")]
fn sheet_round_trip(w: &mut World, rel: String) {
    let sheet_bytes = read_pack_file(w, &rel);
    let sheet_text =
        std::str::from_utf8(&sheet_bytes).unwrap_or_else(|err| panic!("sheet utf-8: {err}"));
    let sheet: spookery_kernel::model::PcSheet =
        serde_saphyr::from_str(sheet_text).unwrap_or_else(|err| panic!("load sheet: {err}"));
    let sheet_emit =
        serde_saphyr::to_string_with_options(&sheet, spookery_kernel::codec::NOW_EMIT_OPTIONS())
            .unwrap_or_else(|err| panic!("emit sheet: {err}"));
    assert_eq!(sheet_emit.as_bytes(), sheet_bytes.as_slice());
}

#[then(expr = "the shard opens")]
fn shard_opens(w: &mut World) {
    spookery_kernel::open_shard(w.pack.as_ref().unwrap())
        .unwrap_or_else(|err| panic!("open_shard: {err}"));
}

fn read_pack_file(w: &World, rel: &str) -> Vec<u8> {
    let what = if rel.ends_with("pc_rowan.yaml") {
        "sheet"
    } else {
        "now"
    };
    fs::read(w.pack.as_ref().unwrap().join(rel)).unwrap_or_else(|err| panic!("read {what}: {err}"))
}
