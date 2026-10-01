use std::fs;

use cucumber::{then, when};
use spookery_kernel::model::{ActorKind, Intent, IntentArgs, Source, Target};
use spookery_tick::{run_tick, Catchup, Reason};

use crate::World;

#[when(expr = "Mira files an idle delete_place proposal for docks_shop")]
fn file_idle_delete_place(w: &mut World) {
    let pack = w.pack.as_ref().unwrap();
    let place_path = pack.join("world/places/docks_shop.md");
    let place_before = fs::read(&place_path).unwrap_or_else(|err| panic!("read place: {err}"));
    let place_before =
        String::from_utf8(place_before).unwrap_or_else(|err| panic!("utf-8 place: {err}"));
    w.scratch.insert("place_before".to_string(), place_before);

    let mut shard =
        spookery_kernel::open_shard(pack).unwrap_or_else(|err| panic!("open_shard: {err}"));
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
}

#[then(expr = "proposal delete_docks_shop.yaml exists")]
fn proposal_exists(w: &mut World) {
    let pack = w.pack.as_ref().unwrap();
    let proposal = pack.join("state/proposals/delete_docks_shop.yaml");
    assert!(proposal.is_file(), "proposal file missing");
}

#[then(expr = "the docks_shop place markdown is unchanged")]
fn place_unchanged(w: &mut World) {
    let pack = w.pack.as_ref().unwrap();
    let place_path = pack.join("world/places/docks_shop.md");
    let place_after = fs::read(&place_path).unwrap_or_else(|err| panic!("read place: {err}"));
    let place_before = w
        .scratch
        .get("place_before")
        .unwrap_or_else(|| panic!("missing place_before"));
    assert_eq!(place_after, place_before.as_bytes(), "place markdown changed");
}
