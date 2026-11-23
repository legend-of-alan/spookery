use cucumber::{given, then, when};

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use spookery_engine::testing::{
    self as support, act_of, place_of, resume, stamp_pack_rev, world_of, write_save, Canon,
};
use spookery_engine::{
    component, ecs, emit_save, entity, load_canon, play_pack_once, ActorId, GlobalId, Meter, Note,
    PlaceId, Presence,
};

use crate::World;

const LEDGER: &str = "The lamp-oil ledger is short.";

/// Put actor `id`, controlled as `control_type` says, in `place`, with vigor `vigor`, the docks
/// pack's other meters at their base, and nothing else.
fn add_actor(
    world: &mut ecs::World,
    id: &str,
    control_type: component::ControlType,
    place: &str,
    vigor: i64,
) {
    let (mut actors, id) = (world.actors_mut(), ActorId::from(id));
    actors.add_actor(&id, control_type, place);
    for (key, base) in [("hp", 8), ("might", 1), ("rest", 3), ("vigor", vigor)] {
        let meter = Meter { base, offset: 0 };
        actors.get_mut::<Meter>(&id).set(key, Some(meter)).unwrap();
    }
}

fn write_text(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .unwrap_or_else(|err| panic!("mkdir {}: {err}", parent.display()));
    }
    fs::write(path, text).unwrap_or_else(|err| panic!("write {}: {err}", path.display()));
}

fn lamp_oil(world: &ecs::World) -> u32 {
    let shop = PlaceId::from("docks_shop");
    let items = world.places().get::<component::Item>(&shop);
    *items.get("lamp_oil").expect("lamp_oil missing")
}

fn flags_of(world: &ecs::World, id: &str) -> Vec<String> {
    assert!(world.actors().contains(&id.into()), "{id} missing");
    let id = id.into();
    let flags = world.actors().get::<component::Flag>(&id);
    flags.iter().map(|(flag, ())| flag.to_string()).collect()
}

fn snapshot(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
        let mut entries: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap())
            .collect();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, root, out);
            } else {
                let rel = path.strip_prefix(root).unwrap().to_path_buf();
                out.push((rel, fs::read(&path).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out
}

fn door_note() -> Note {
    Note {
        to_ids: BTreeSet::from(["rowan".into()]),
        place_id: Some("docks_shop".into()),
        fact_ids: BTreeSet::new(),
        text: "Back at dusk. Oil on the shelf.".to_string(),
    }
}

/// Leave Mira's door note, as her habit would.
fn leave_door_note(world: &mut ecs::World) {
    let mut globals = world.globals_mut();
    let left = globals
        .get_mut::<Note>(&GlobalId)
        .leave("note_mira_door", door_note());
    left.unwrap();
}

/// The ids of the notes left in the world, in order.
fn note_ids(world: &ecs::World) -> Vec<&str> {
    let notes = world.globals().get::<Note>(&GlobalId);
    notes.iter().map(|(id, _)| id).collect()
}

/// Rowan is the one seated player.
fn rowan() -> Presence {
    Presence {
        players: 1,
        actors_present: vec!["rowan".into()],
    }
}

fn evening_note() {
    let pack = support::copy_pack("docks", "evening");
    play_pack_once(&pack).unwrap_or_else(|err| panic!("round: {err}"));
    let world = world_of(&pack);
    assert_eq!(lamp_oil(&world), 4);
    assert_eq!(place_of(&world, "mira"), "mira_rooms");
    let notes = world.globals().get::<Note>(&GlobalId);
    let door = notes.get("note_mira_door").expect("the door note");
    assert_eq!(note_ids(&world), ["note_mira_door"]);
    assert_eq!(door.place_id, Some("docks_shop".into()));
    let _ = fs::remove_dir_all(&pack);
}

fn morning_restock() {
    let pack = support::copy_pack("docks", "morning");
    {
        let opened = resume(&pack);
        let mut world = opened.world().clone();
        drop(opened);
        world.world_time = 600;
        leave_door_note(&mut world);
        world
            .actors_mut()
            .get_mut::<component::Place>(&"rowan".into())
            .set("docks_street")
            .unwrap();
        write_save(&pack, &world);
    }
    assert_eq!(place_of(&world_of(&pack), "mira"), "mira_rooms");
    play_pack_once(&pack).unwrap_or_else(|err| panic!("round: {err}"));
    let world = world_of(&pack);
    assert_eq!(lamp_oil(&world), 8);
    assert_eq!(place_of(&world, "mira"), "mira_rooms");
    assert_eq!(note_ids(&world), ["note_mira_door"]);
    let _ = fs::remove_dir_all(&pack);
}

fn capped_restock() -> u32 {
    let pack = support::copy_pack("docks", "capped");
    {
        let opened = resume(&pack);
        let mut world = opened.world().clone();
        drop(opened);
        world.world_time = 600;
        leave_door_note(&mut world);
        world
            .actors_mut()
            .get_mut::<component::Place>(&"rowan".into())
            .set("docks_street")
            .unwrap();
        let entry = ("docks_shop".into(), "lamp_oil".to_string());
        let stock = world.places().get::<component::Item>(&entry.0);
        let held = i64::from(stock.get(&entry.1).copied().unwrap_or(0));
        let mut places = world.places_mut();
        let mut shop = places.get_mut::<component::Item>(&entry.0);
        shop.shift("lamp_oil", 6 - held, None).unwrap();
        write_save(&pack, &world);
    }
    play_pack_once(&pack).unwrap_or_else(|err| panic!("round: {err}"));
    let oil = lamp_oil(&world_of(&pack));
    let _ = fs::remove_dir_all(&pack);
    oil
}

fn off_hours_hold() {
    let pack = support::copy_pack("docks", "off-hours");
    {
        let opened = resume(&pack);
        let mut world = opened.world().clone();
        drop(opened);
        leave_door_note(&mut world);
        write_save(&pack, &world);
    }
    play_pack_once(&pack).unwrap_or_else(|err| panic!("round: {err}"));
    let world = world_of(&pack);
    assert_eq!(lamp_oil(&world), 4);
    assert_eq!(note_ids(&world), ["note_mira_door"]);
    let _ = fs::remove_dir_all(&pack);
}

fn player_in_the_shop() {
    let pack = support::copy_pack("docks", "present");
    {
        let opened = resume(&pack);
        let mut world = opened.world().clone();
        drop(opened);
        world.world_time = 600;
        leave_door_note(&mut world);
        world
            .actors_mut()
            .get_mut::<component::Place>(&"mira".into())
            .set("docks_shop")
            .unwrap();
        write_save(&pack, &world);
    }
    let mut opened = resume(&pack);
    let wall = opened.world().last_round_at.clone();
    opened
        .play(&rowan(), &wall)
        .unwrap_or_else(|err| panic!("round: {err}"));
    let world = world_of(&pack);
    assert_eq!(lamp_oil(&world), 4);
    assert_eq!(note_ids(&world), ["note_mira_door"]);
    assert_eq!(act_of(&world, "mira").last_act_round, 0);
    let _ = fs::remove_dir_all(&pack);
}

fn queue_gm_intent(opened: &mut spookery_engine::Session, id: &str, actor: &str, move_id: &str) {
    opened
        .queue_gm_intent(
            &spookery_engine::Presence::default(),
            spookery_protocol::QueueArgs {
                id: None,
                round_index: None,
                actor_id: Some(actor.to_string()),
                move_id: Some(move_id.to_string()),
                target: None,
                message_to: None,
                rationale_code: String::new(),
            },
            || Ok(id.to_string()),
        )
        .unwrap();
}

fn gm_restocks() {
    let pack = support::copy_pack("docks", "gm-restock");
    {
        let opened = resume(&pack);
        let mut world = opened.world().clone();
        drop(opened);
        world.world_time = 600;
        leave_door_note(&mut world);
        world
            .actors_mut()
            .get_mut::<component::Place>(&"mira".into())
            .set("docks_shop")
            .unwrap();
        write_save(&pack, &world);
    }
    let mut opened = resume(&pack);
    let wall = opened.world().last_round_at.clone();
    queue_gm_intent(&mut opened, "gm-1", "mira", "restock_lamp_oil");
    opened.play(&rowan(), &wall).unwrap();
    assert_eq!(lamp_oil(opened.world()), 8);
    assert_eq!(note_ids(opened.world()), ["note_mira_door"]);
    let _ = fs::remove_dir_all(&pack);
}

/// With room for one intent, the door note for seated Rowan lands and Bram's wait does not.
fn claimed_note_sorts_first() {
    let pack = support::copy_pack("docks", "claimed");
    let budget = pack.join("rules/budget.yaml");
    let text = fs::read_to_string(&budget).unwrap();
    fs::write(
        &budget,
        text.replace("max_intents_applied: 12", "max_intents_applied: 1"),
    )
    .unwrap();
    stamp_pack_rev(&pack);
    {
        let opened = resume(&pack);
        let mut world = opened.world().clone();
        drop(opened);
        world
            .actors_mut()
            .get_mut::<component::Place>(&"mira".into())
            .set("docks_shop")
            .unwrap();
        write_save(&pack, &world);
    }
    let mut opened = resume(&pack);
    let wall = opened.world().last_round_at.clone();
    for (id, actor, move_id) in [
        ("gm-1", "bram", "wait"),
        ("gm-2", "mira", "leave_door_note"),
    ] {
        queue_gm_intent(&mut opened, id, actor, move_id);
    }
    opened.play(&rowan(), &wall).unwrap();
    let world = opened.world();
    let notes = world.globals().get::<Note>(&GlobalId);
    let door = notes.get("note_mira_door").expect("the door note");
    assert_eq!(door.to_ids, BTreeSet::from(["rowan".into()]));
    assert_eq!(
        act_of(world, "bram").last_act_round,
        0,
        "bram's wait landed"
    );
    drop(opened);
    let _ = fs::remove_dir_all(&pack);
}

fn catch_up_to_cap() {
    catch_up_for(72, 12);
}

fn catch_up_for(hours: u64, rounds: u64) {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let pack = support::fresh_dir("catch_up");
    let _ = fs::remove_dir_all(&pack);
    let rules = manifest.join("../../tests/packs/docks/rules");
    write_text(
        &pack.join("pack.yaml"),
        "\
schema: pack
id: ex
title: Ex
system: ex
start:
  place: gate
",
    );
    write_text(
        &pack.join("world/map.yaml"),
        "schema: map
paths: []
",
    );
    write_text(
        &pack.join("world/places/gate.md"),
        "\
---
schema: place
id: gate
name: Gate
facts:
  - id: fact_gate
    text: The gate is shut.
---
A gate.
",
    );
    write_text(
        &pack.join("cast/rowan.md"),
        "\
---
schema: cast
id: rowan
control_type: pc_template
name: Rowan
---
Rowan.
",
    );
    for file in ["verbs.yaml", "meters.yaml", "budget.yaml"] {
        fs::copy(rules.join(file), {
            let dest = pack.join("rules").join(file);
            fs::create_dir_all(dest.parent().unwrap()).unwrap();
            dest
        })
        .unwrap_or_else(|err| panic!("copy {file}: {err}"));
    }
    let rev = spookery_engine::pack_rev(&pack).unwrap();
    let mut world = empty_save(load_canon(&pack).unwrap(), "ex", &rev);
    add_actor(&mut world, "rowan", component::ControlType::Pc, "gate", 1);
    write_text(&pack.join("state/save.yaml"), &emit_save(&world).unwrap());
    let place_before = fs::read(pack.join("world/places/gate.md")).unwrap();
    let cast_before = fs::read(pack.join("cast/rowan.md")).unwrap();
    let mut opened = resume(&pack);
    let wall = opened.world().last_round_at.clone();
    opened
        .catch_up(&wall, hours)
        .unwrap_or_else(|err| panic!("catch-up: {err}"));
    let saved = spookery_engine::Session::peek(&pack).unwrap_or_else(|err| panic!("peek: {err}"));
    let world = saved.world();
    assert_eq!(world.state_rev, rounds, "{hours} hours");
    assert_eq!(world.round_index, rounds);
    assert_eq!(world.world_time, rounds * 60);
    assert_eq!(world.last_round_at, "2026-01-01T00:00:00Z");
    assert_eq!(
        fs::read(pack.join("world/places/gate.md")).unwrap(),
        place_before
    );
    assert_eq!(fs::read(pack.join("cast/rowan.md")).unwrap(), cast_before);
    let _ = fs::remove_dir_all(&pack);
}

fn a_busy_npc_skips() {
    let pack = support::copy_pack("docks", "busy");
    {
        let opened = resume(&pack);
        let mut world = opened.world().clone();
        drop(opened);
        world.world_time = 600;
        leave_door_note(&mut world);
        let mut actors = world.actors_mut();
        actors
            .get_mut::<component::Act>(&"mira".into())
            .set_busy(u64::MAX, None)
            .unwrap();
        world
            .actors_mut()
            .get_mut::<component::Place>(&"rowan".into())
            .set("docks_street")
            .unwrap();
        write_save(&pack, &world);
    }
    play_pack_once(&pack).unwrap_or_else(|err| panic!("round: {err}"));
    let world = world_of(&pack);
    assert_eq!(lamp_oil(&world), 4);
    assert_eq!(act_of(&world, "mira").last_act_round, 0);
    assert_eq!(act_of(&world, "bram").last_act_round, 0);
    let _ = fs::remove_dir_all(&pack);
}

fn budget_caps() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let pack = support::fresh_dir("budget");
    let _ = fs::remove_dir_all(&pack);
    let rules = manifest.join("../../tests/packs/docks/rules");
    write_text(
        &pack.join("pack.yaml"),
        "\
schema: pack
id: crowd
title: Crowd
system: crowd
start:
  place: yard
",
    );
    write_text(
        &pack.join("world/map.yaml"),
        "schema: map
paths: []
",
    );
    write_text(
        &pack.join("world/places/yard.md"),
        "\
---
schema: place
id: yard
name: Yard
facts:
  - id: fact_yard
    text: The yard is open.
---
A yard.
",
    );
    let mut npcs = Vec::new();
    for index in 1..=50 {
        let id = format!("n{index}");
        write_text(
            &pack.join(format!("cast/{id}.md")),
            &format!(
                "\
---
schema: cast
id: {id}
control_type: npc
idle:
  eligible: true
rules:
  - id: hold
    when: habit
    do: []
---
{id}
"
            ),
        );
        npcs.push(id);
    }
    for file in ["verbs.yaml", "meters.yaml", "budget.yaml"] {
        let dest = pack.join("rules").join(file);
        fs::create_dir_all(dest.parent().unwrap()).unwrap();
        fs::copy(rules.join(file), &dest).unwrap();
    }
    let rev = spookery_engine::pack_rev(&pack).unwrap();
    let mut world = empty_save(load_canon(&pack).unwrap(), "crowd", &rev);
    for id in &npcs {
        add_actor(&mut world, id, component::ControlType::Npc, "yard", 1);
    }
    write_text(&pack.join("state/save.yaml"), &emit_save(&world).unwrap());
    play_pack_once(&pack).unwrap_or_else(|err| panic!("round: {err}"));
    let world = world_of(&pack);
    assert_eq!(world.round_index, 1);
    assert_eq!(
        world
            .actors()
            .query::<&component::ControlType>()
            .iter()
            .filter(|control_type| !matches!(control_type, component::ControlType::Pc))
            .count(),
        50
    );
    let acted = world
        .actors()
        .query::<&component::Act>()
        .iter()
        .filter(|act| act.last_act_round == 1)
        .count();
    assert!((1..=8).contains(&acted), "acted {acted}");
    let _ = fs::remove_dir_all(&pack);
}

fn a_zero_budget_strips_fact_bodies() {
    let pack = support::copy_pack("docks", "scope");
    let before = snapshot(&pack);
    let opened = spookery_engine::Session::open(&pack).unwrap();
    let call = |viewer_id: &str, budget: usize| {
        opened.perceive_trimmed(&Presence::default(), viewer_id, budget)
    };
    let kernel = opened.perceive(&Presence::default(), "mira").unwrap();
    assert_eq!(call("mira", 800).unwrap(), kernel);
    let capped = call("mira", 0).unwrap();
    assert!(kernel
        .known_facts
        .iter()
        .any(|fact| !fact.owner_body.is_empty()));
    assert_eq!(capped.known_facts.len(), kernel.known_facts.len());
    assert!(capped
        .known_facts
        .iter()
        .all(|fact| fact.owner_body.is_empty()));
    let ledger = capped
        .known_facts
        .iter()
        .find(|fact| fact.id == "fact_ledger_short")
        .unwrap();
    assert_eq!(ledger.text, LEDGER);
    assert!(!capped
        .unknown_mentions
        .iter()
        .any(|id| id == "fact_ledger_short"));
    assert!(capped
        .unknown_mentions
        .iter()
        .any(|id| id == "fact_room_lamp"));
    let unscoped = spookery_engine::Reason::UnscopedPerceive;
    assert_eq!(call("gm", 800).unwrap_err(), unscoped);
    drop(opened);
    assert_eq!(snapshot(&pack), before);
    let _ = fs::remove_dir_all(&pack);
}

fn collect_markdown_files(dir: &Path, out: &mut Vec<PathBuf>) {
    if !dir.exists() {
        return;
    }
    let mut entries: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap())
        .collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect_markdown_files(&path, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("md") {
            out.push(path);
        }
    }
}

fn strip_bodies() {
    let pack = support::copy_pack("docks", "strip");
    let mut files = Vec::new();
    collect_markdown_files(&pack.join("cast"), &mut files);
    collect_markdown_files(&pack.join("world"), &mut files);
    assert!(!files.is_empty());
    for path in &files {
        let text = fs::read_to_string(path).unwrap();
        let stripped = spookery_engine::testing::strip_body(&text).unwrap();
        assert!(
            !stripped.contains("Mira keeps the shop"),
            "{}",
            path.display()
        );
        assert!(!stripped.contains("Bram walks"), "{}", path.display());
        fs::write(path, stripped).unwrap();
    }
    let shop = fs::read_to_string(pack.join("world/places/docks_shop.md")).unwrap();
    assert!(shop.contains(LEDGER), "frontmatter secret removed");
    assert!(!fs::read_to_string(pack.join("cast/aldric.md"))
        .unwrap()
        .contains(LEDGER));
    stamp_pack_rev(&pack);
    resume(&pack);
    play_pack_once(&pack).unwrap_or_else(|err| panic!("stripped pack did not tick: {err}"));
    let perceive = format!(
        "{:?}",
        resume(&pack)
            .perceive(&Presence::default(), "aldric")
            .unwrap()
    );
    assert!(!perceive.contains(LEDGER));
    let _ = fs::remove_dir_all(&pack);
}

fn patch_mira_flags_and_place(pack: &Path, flags: &str) {
    let path = pack.join("cast/mira.md");
    let text = fs::read_to_string(&path).unwrap();
    let next = text.replacen(
        "idle:\n  eligible: true\n",
        &format!("idle:\n  eligible: true\nflags:\n{flags}"),
        1,
    );
    assert_ne!(next, text, "mira round block was not patched");
    fs::write(&path, next).unwrap();
    let now_path = pack.join("state/save.yaml");
    let save = spookery_engine::parse_save(&fs::read_to_string(&now_path).unwrap()).unwrap();
    let canon = load_canon(pack).unwrap_or_else(|err| panic!("load: {err:?}"));
    let mut world = ecs::World::new(save, Arc::new(canon));
    world
        .actors_mut()
        .get_mut::<component::Place>(&"mira".into())
        .set("docks_shop")
        .unwrap();
    write_save(pack, &world);
}

fn play_mark(pack: &Path) -> ecs::World {
    let mut opened = resume(pack);
    let wall = opened.world().last_round_at.clone();
    opened
        .queue_player_move(&rowan(), &"rowan".into(), "mark", Some("mira"))
        .unwrap();
    let played = opened.commit_then_fast_forward(&rowan(), &wall).unwrap();
    assert!(!played.is_empty(), "mark mira did not commit");
    opened.world().clone()
}

fn effects_run_on_both_outcomes_and_lists_run_on_one() {
    for (label, difficulty, success) in [("effects-pass", 0, true), ("effects-fail", 14, false)] {
        let pack = support::copy_pack("docks", label);
        let mut moves = fs::read_to_string(pack.join("rules/verbs.yaml")).unwrap();
        moves.push_str(&format!(
            "\n  - id: mark\n    when:\n      verb: mark\n      target: actor\n    check:\n      total: 2d6 + self.vigor\n      success_at: {difficulty}\n    do:\n      - flag:\n          of: target\n          key: marked\n          value: true\n",
        ));
        fs::write(pack.join("rules/verbs.yaml"), moves).unwrap();
        patch_mira_flags_and_place(&pack, "    - marked\n");
        stamp_pack_rev(&pack);
        let world = play_mark(&pack);
        assert_eq!(world.world_time, 1085);
        assert_eq!(
            flags_of(&world, "mira"),
            vec!["marked".to_string()],
            "{label} success {success}"
        );
        let _ = fs::remove_dir_all(&pack);
    }

    for (label, difficulty, flag) in [("lists-pass", 0, "marked"), ("lists-fail", 14, "missed")] {
        let pack = support::copy_pack("docks", label);
        let mut moves = fs::read_to_string(pack.join("rules/verbs.yaml")).unwrap();
        moves.push_str(&format!(
            "\n  - id: mark\n    when:\n      verb: mark\n      target: actor\n    check:\n      total: 2d6 + self.vigor\n      success_at: {difficulty}\n    success:\n      - flag:\n          of: target\n          key: marked\n          value: true\n    miss:\n      - flag:\n          of: target\n          key: missed\n          value: true\n",
        ));
        fs::write(pack.join("rules/verbs.yaml"), moves).unwrap();
        patch_mira_flags_and_place(&pack, "    - marked\n    - missed\n");
        stamp_pack_rev(&pack);
        let world = play_mark(&pack);
        assert_eq!(flags_of(&world, "mira"), vec![flag.to_string()], "{label}");
        assert_eq!(world.world_time, 1085);
        let _ = fs::remove_dir_all(&pack);
    }
}

fn rowans_debt(pack: &Path) -> i64 {
    let rowan = entity::Id::Actor("rowan".into());
    world_of(pack)
        .entities()
        .get::<component::Meter>(&rowan)
        .value("assassination_debt")
        .unwrap_or_else(|err| panic!("assassination_debt: {err}"))
}

#[given(expr = "rowan's {string} meter is {int}")]
fn rowans_meter_is(world: &mut World, key: String, value: i64) {
    assert_eq!(key, "assassination_debt");
    let pack = support::copy_pack("docks", "debt");
    assert_eq!(rowans_debt(&pack), value);
    world.pack = Some(pack);
}

#[given(expr = "the rule that advances it has {string}")]
fn the_debt_rule_has(world: &mut World, guard: String) {
    assert_eq!(guard, "if: players >= 1");
    let pack = world.pack();
    let rule = "\
rules:
  - id: debt_grows
    when: habit
    if:
      - players:
          op: \">=\"
          value: 1
    do:
      - meter:
          of: self
          key: assassination_debt
          add: 1
---
Rowan";
    let path = pack.join("cast/pc-rowan.md");
    let text = fs::read_to_string(&path).unwrap();
    write_text(&path, &text.replacen("---\nRowan", rule, 1));
    stamp_pack_rev(pack);
    let canon = load_canon(pack).unwrap_or_else(|err| panic!("load: {err:?}"));
    let rules = canon.rules.ids_in_order();
    assert!(rules.contains(&"cast:rowan/debt_grows"), "{rules:?}");
}

#[when(expr = "twelve empty rounds pass")]
fn twelve_empty_rounds_pass(world: &mut World) {
    for round in 1..=12 {
        play_pack_once(world.pack()).unwrap_or_else(|err| panic!("round {round}: {err}"));
    }
    assert_eq!(
        world_of(world.pack()).round_index,
        12,
        "twelve empty-table rounds commit"
    );
}

#[then(expr = "assassination_debt is still {int}")]
fn the_debt_is_still(world: &mut World, value: i64) {
    assert_eq!(rowans_debt(world.pack()), value);
}

#[when(expr = "the evening round runs with Rowan away")]
fn the_evening_round_runs(_world: &mut World) {
    evening_note();
}

#[when(expr = "the morning round runs with the note already left")]
fn the_morning_round_runs(_world: &mut World) {
    morning_restock();
}

#[when(expr = "the morning round runs and the shelf already holds 6 lamp oil")]
fn the_morning_round_caps_the_shelf(world: &mut World) {
    world.scratch = capped_restock().to_string();
}

#[then(expr = "the shop holds 8 lamp oil")]
fn the_shop_holds_8_lamp_oil(world: &mut World) {
    assert_eq!(world.scratch, "8");
}

#[when(expr = "an off-hours round runs with the note already left")]
fn an_off_hours_round_runs(_world: &mut World) {
    off_hours_hold();
}

#[when(expr = "Rowan stands with Mira during shop hours")]
fn rowan_stands_with_mira(_world: &mut World) {
    player_in_the_shop();
}

#[when(expr = "the GM queues Mira's restock in front of Rowan")]
fn the_gm_queues_miras_restock(_world: &mut World) {
    gm_restocks();
}

#[when(expr = "the GM queues the door note ahead of a wait")]
fn the_gm_queues_the_door_note(_world: &mut World) {
    claimed_note_sorts_first();
}

#[when(expr = "seventy-two hours of catch-up run")]
fn seventy_two_hours_of_catch_up_run(_world: &mut World) {
    catch_up_to_cap();
}

#[when(expr = "one hour of catch-up runs")]
fn one_hour_of_catch_up(_world: &mut World) {
    catch_up_for(1, 8);
}

#[when(expr = "Mira is busy and a round runs")]
fn mira_is_busy(_world: &mut World) {
    a_busy_npc_skips();
}

#[given(expr = "a docks table")]
fn a_docks_table(world: &mut World) {
    let pack = support::copy_pack("docks", "crash");
    world.scratch = fs::read_to_string(pack.join("state/save.yaml")).unwrap();
    world.pack = Some(pack);
}

#[when(expr = "a player queues take lamp_oil and the host stops before a round")]
fn a_player_queues_take_and_the_host_stops(world: &mut World) {
    let mut host = resume(world.pack());
    host.queue_player_move(&rowan(), &"rowan".into(), "take", Some("lamp_oil"))
        .unwrap();
    assert_eq!(host.queued_intents().len(), 1);
}

#[when(expr = "a save.yaml.tmp and a bundle-0000.jsonl.gz.tmp are left under state\\/")]
fn stray_temp_files_are_left(world: &mut World) {
    let state = world.pack().join("state");
    write_text(&state.join("save.yaml.tmp"), "half");
    write_text(&state.join("journal/bundle-0000.jsonl.gz.tmp"), "half");
}

#[when(expr = "the table is opened again")]
fn the_table_is_opened_again(world: &mut World) {
    drop(resume(world.pack()));
}

#[then(expr = "the save bytes are unchanged")]
fn the_save_bytes_are_unchanged(world: &mut World) {
    let save = fs::read(world.pack().join("state/save.yaml")).unwrap();
    assert_eq!(save, world.scratch.as_bytes());
}

#[then(expr = "the next host opens at the same state_rev with the same lamp oil")]
fn the_next_host_opens_at_the_same_state(world: &mut World) {
    let before = spookery_engine::parse_save(&world.scratch).unwrap();
    let host = resume(world.pack());
    assert_eq!(host.world().state_rev, before.state_rev);
    let before = ecs::World::new(before, host.world().canon().clone());
    assert_eq!(lamp_oil(host.world()), lamp_oil(&before));
}

#[then(expr = "no .tmp file remains under state\\/")]
fn no_tmp_file_remains_under_state(world: &mut World) {
    let left: Vec<PathBuf> = snapshot(&world.pack().join("state"))
        .into_iter()
        .map(|(path, _)| path)
        .filter(|path| path.extension().is_some_and(|ext| ext == "tmp"))
        .collect();
    assert!(left.is_empty(), "{left:?}");
}

#[when(expr = "fifty NPCs share one round")]
fn fifty_npcs_share_one_round(_world: &mut World) {
    budget_caps();
}

#[when(expr = "an npc perceive is called with a zero budget")]
fn an_npc_perceive_is_called(_world: &mut World) {
    a_zero_budget_strips_fact_bodies();
}

#[when(expr = "prose is stripped")]
fn prose_is_stripped(_world: &mut World) {
    strip_bodies();
}

#[when(expr = "effect rows and outcome lists are played")]
fn effect_rows_and_outcome_lists_are_played(_world: &mut World) {
    effects_run_on_both_outcomes_and_lists_run_on_one();
}

/// A save with no actors on `canon`, for world `world_id` at pack revision `rev`.
fn empty_save(canon: Canon, world_id: &str, rev: &str) -> ecs::World {
    let rules_rev = spookery_engine::testing::RULES_REV;
    let text = format!(
        "\
schema: save
world_id: {world_id}
pack_rev: \"{rev}\"
rules_rev: {rules_rev}
state_rev: 0
round_index: 0
world_time: 0
last_round_at: \"2026-01-01T00:00:00Z\"
actors: {{}}
"
    );
    let save = spookery_engine::parse_save(&text).unwrap_or_else(|err| panic!("save: {err:?}"));
    ecs::World::new(save, Arc::new(canon))
}
