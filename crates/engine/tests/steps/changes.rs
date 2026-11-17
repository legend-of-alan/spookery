use std::fs;

use cucumber::{given, then, when};
use spookery_engine::testing::{self, stamp_pack_rev};
use spookery_engine::Presence;

use crate::World;

#[given(expr = "harbormouse has a hidden meter")]
fn harbormouse_has_a_hidden_meter(world: &mut World) {
    let pack = world.pack();
    let mut meters = fs::read_to_string(pack.join("rules/meters.yaml")).unwrap();
    meters.push_str(
        "  - id: omen\n    min: 0\n    base_cap: 5\n    max: 5\n    base: 1\n    hidden: true\n",
    );
    fs::write(pack.join("rules/meters.yaml"), meters).unwrap();
    stamp_pack_rev(pack);
    spookery_engine::migrate_pack(pack).unwrap_or_else(|err| panic!("migrate: {err}"));
}

#[when(expr = "harbormouse looks at their view")]
fn harbormouse_looks_at_their_view(world: &mut World) {
    let pack = world.pack().to_path_buf();
    let session = testing::resume(&pack);
    let seated = Presence {
        players: 1,
        actors_present: vec!["harbormouse".into()],
    };
    let view = session
        .perceive(&seated, "harbormouse")
        .unwrap_or_else(|err| panic!("perceive: {}", err.as_str()));
    world.seen = view
        .meters
        .iter()
        .map(|meter| format!("{}: {}", meter.id, meter.value))
        .collect::<Vec<_>>()
        .join("\n");
}

#[then(expr = "harbormouse sees their own hp and might")]
fn harbormouse_sees_hp_and_might(world: &mut World) {
    assert!(world.seen.contains("hp: 8"), "{}", world.seen);
    assert!(world.seen.contains("might: 30"), "{}", world.seen);
    assert!(!world.seen.contains("might: 2"), "{}", world.seen);
}

#[then(expr = "harbormouse does not see a hidden meter")]
fn harbormouse_does_not_see_a_hidden_meter(world: &mut World) {
    assert!(
        !world.seen.to_lowercase().contains("omen"),
        "{}",
        world.seen
    );
}
