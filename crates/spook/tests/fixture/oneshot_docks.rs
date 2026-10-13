use std::fs;
use std::path::PathBuf;

pub(crate) fn docks_save_round_trips() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packs/oneshot-docks");
    let rev =
        spookery_kernel::canon::pack_rev(&root).unwrap_or_else(|err| panic!("pack_rev: {err}"));

    let now_bytes =
        fs::read(root.join("state/save.yaml")).unwrap_or_else(|err| panic!("read now: {err}"));
    let now_text = std::str::from_utf8(&now_bytes).unwrap_or_else(|err| panic!("now utf-8: {err}"));
    let now = spookery_kernel::store::codec::load_now(now_text)
        .unwrap_or_else(|err| panic!("load now: {err:?}"));
    let emitted = spookery_kernel::store::codec::emit_now(&now)
        .unwrap_or_else(|err| panic!("emit now: {err}"));
    assert_eq!(emitted.as_bytes(), now_bytes.as_slice());
    assert_eq!(now.pack_rev, rev);
    assert_eq!(now.state_rev, 0);
    assert_eq!(now.world_time, 1080);
    assert_eq!(now.actors.get("pc_rowan").unwrap().place(), "docks_shop");
    assert!(!now_text.contains("session_ids"));

    spookery_kernel::World::open(&root).unwrap_or_else(|err| panic!("World::open: {err}"));
}
