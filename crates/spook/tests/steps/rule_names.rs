use std::fs;
use std::path::Path;

use cucumber::{given, then, when};
use spookery_engine::Reason;

use crate::support::{copy_pack, insert_nell, stamp_pack_rev};
use crate::World;

use super::host;

#[given(expr = "a docks table where mark needs a knife")]
async fn mark_needs_a_knife(world: &mut World) {
    let pack = copy_pack("docks", "item");
    add_knife_and_mark_rule(&pack);
    insert_nell(&pack);
    stamp_pack_rev(&pack);
    host::listen(world, &pack).await;
    host::seat_rowan(world, &pack).await;
    world.pack = Some(pack);
}

#[when(expr = "the verbs file gains a shout row")]
fn gains_a_shout_row(world: &mut World) {
    append_verbs(
        world,
        "\n  - id: mark\n    when: habit\n    do:\n      - shout:\n          to: aldric\n",
    );
}

fn append_verbs(world: &mut World, extra: &str) {
    let pack = host::pack(world);
    let mut verbs = fs::read_to_string(pack.join("rules/verbs.yaml")).unwrap();
    verbs.push_str(extra);
    fs::write(pack.join("rules/verbs.yaml"), verbs).unwrap();
}

#[then(expr = "the load refusal is SCHEMA")]
fn the_load_refusal_is_schema(world: &mut World) {
    assert_eq!(
        spookery_engine::diagnostic::find_reason(
            &spookery_engine::canon::load(host::pack(world)).unwrap_err()
        ),
        Reason::Schema
    );
}

fn add_knife_and_mark_rule(pack: &Path) {
    let item = pack.join("world/items/knife.md");
    fs::create_dir_all(item.parent().unwrap()).unwrap();
    fs::write(
        &item,
        "---\nschema: item\nid: knife\nname: Knife\ntags: []\nrules: []\n---\n\nA knife.\n",
    )
    .unwrap();
    let mut verbs = fs::read_to_string(pack.join("rules/verbs.yaml")).unwrap();
    verbs.push_str(
        "\n  - id: mark\n    when:\n      verb: mark\n      target: actor\n    if:\n      - has_item:\n          subject: self\n          id: knife\n    check:\n      total: 2d6 + self.vigor\n      success_at: 0\n    success:\n      - flag:\n          of: target\n          id: marked\n          value: true\n    miss:\n      - flag:\n          of: target\n          id: missed\n          value: true\n",
    );
    fs::write(pack.join("rules/verbs.yaml"), verbs).unwrap();
}
