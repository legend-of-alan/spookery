use std::fs;
use std::path::PathBuf;

use spookery_kernel::model::{ActorKind, Intent, IntentArgs, Source, Target};
use spookery_tick::{run_tick, Catchup, Reason};

use super::support::copy_dir;

#[test]
fn v0_t10_proposal_only() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixture = manifest.join("packs/oneshot-docks");
    let pack = std::env::temp_dir().join(format!(
        "spook-tick10-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&pack);
    copy_dir(&fixture, &pack);

    let place_path = pack.join("world/places/docks_shop.md");
    let place_before = fs::read(&place_path).unwrap_or_else(|err| panic!("read place: {err}"));

    let mut shard =
        spookery_kernel::open_shard(&pack).unwrap_or_else(|err| panic!("open_shard: {err}"));
    assert_eq!(shard.now().presence.humans, 0, "idle shard");
    let wall = shard.now().last_tick_at.clone();
    shard.queue_intent(Intent {
        id: "idle-delete-place".to_string(),
        tick_index: shard.now().tick_index + 1,
        actor_id: "mira".to_string(),
        actor_kind: ActorKind::Npc,
        source: Source::Kernel,
        move_name: "file_proposal".to_string(),
        target: Target::Empty,
        args: IntentArgs {
            proposal_id: Some("delete_docks_shop".to_string()),
            kind: Some("delete_place".to_string()),
            target: Some("docks_shop".to_string()),
            summary: Some("delete place".to_string()),
            ..IntentArgs::default()
        },
        rationale_code: String::new(),
    });

    run_tick(&mut shard, Reason::Manual, &wall, Catchup::none())
        .unwrap_or_else(|err| panic!("idle tick: {err}"));

    let proposal = pack.join("state/proposals/delete_docks_shop.yaml");
    assert!(proposal.is_file(), "proposal file missing");
    let place_after = fs::read(&place_path).unwrap_or_else(|err| panic!("read place: {err}"));
    assert_eq!(place_after, place_before, "place markdown changed");

    let _ = fs::remove_dir_all(&pack);
}
