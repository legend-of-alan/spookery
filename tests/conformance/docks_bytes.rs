use std::fs;
use std::path::PathBuf;

const PACK_REV: &str = "blake2s:311fba8fa0b3678adfe606ba8113246ffcee6886a640138bf2adb015ee753ef4";
const NOW_BLAKE2S: &str = "75c31f56b61d32b5c02fa1bb655262d006e608b7a57d0775a3e74742667a45ac";
const SHEET_BLAKE2S: &str = "0bc253962ff105876e0687c3649959cc58735e43b02f6ec43ca13167894f0e7b";

#[test]
fn oneshot_docks_matches_section12() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("packs/oneshot-docks");
    let rev = spookery_kernel::canon::pack_rev(&root).unwrap_or_else(|err| panic!("pack_rev: {err}"));
    assert_eq!(rev, PACK_REV);

    let now_bytes = fs::read(root.join("state/now.yaml")).unwrap_or_else(|err| panic!("read now: {err}"));
    assert_eq!(now_bytes.len(), 2006);
    assert_eq!(spookery_kernel::blake2s_hex(&now_bytes), NOW_BLAKE2S);
    let now_text = std::str::from_utf8(&now_bytes).unwrap_or_else(|err| panic!("now utf-8: {err}"));
    let now = spookery_kernel::codec::load_now(now_text).unwrap_or_else(|err| panic!("load now: {err}"));
    let emitted = spookery_kernel::codec::emit_now(&now).unwrap_or_else(|err| panic!("emit now: {err}"));
    assert_eq!(emitted.as_bytes(), now_bytes.as_slice());
    assert_eq!(now.pack_rev, PACK_REV);

    let sheet_bytes = fs::read(root.join("state/actors/pc_rowan.yaml"))
        .unwrap_or_else(|err| panic!("read sheet: {err}"));
    assert_eq!(sheet_bytes.len(), 118);
    assert_eq!(spookery_kernel::blake2s_hex(&sheet_bytes), SHEET_BLAKE2S);
    let sheet_text = std::str::from_utf8(&sheet_bytes).unwrap_or_else(|err| panic!("sheet utf-8: {err}"));
    let sheet: spookery_kernel::model::PcSheet =
        serde_saphyr::from_str(sheet_text).unwrap_or_else(|err| panic!("load sheet: {err}"));
    let sheet_emit = serde_saphyr::to_string_with_options(&sheet, spookery_kernel::codec::NOW_EMIT_OPTIONS())
        .unwrap_or_else(|err| panic!("emit sheet: {err}"));
    assert_eq!(sheet_emit.as_bytes(), sheet_bytes.as_slice());

    spookery_kernel::open_shard(&root).unwrap_or_else(|err| panic!("open_shard: {err}"));
}
