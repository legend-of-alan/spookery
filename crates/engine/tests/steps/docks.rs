use cucumber::{given, then, when};

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use spookery_engine::testing::{
    self as support, act_of, insert_nell, journal::list_round_keys, place_of,
    read_round as round_value, resume, stamp_pack_rev, state_of, write_save, Canon,
};
use spookery_engine::{
    component, emit_save, entity, load_canon, play_pack_once, ActorId, Meter, MoveReport, Note,
    Presence, Save, State,
};
use spookery_protocol::Outcome;

use crate::World;

const LEDGER: &str = "The lamp-oil ledger is short.";

/// Put actor `id`, controlled as `control_type` says, in `place`, with grit `grit` and rest 3 and nothing else.
fn add_actor(
    state: &mut State,
    id: &str,
    control_type: component::ControlType,
    place: &str,
    grit: i64,
) {
    let (mut actors, id) = (state.actors_mut(), ActorId::from(id));
    actors.add_actor(&id, control_type, place);
    for (key, base) in [("grit", grit), ("rest", 3)] {
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

fn lamp_oil(state: &Save) -> u32 {
    state
        .places()
        .items
        .get(&("docks_shop".into(), "lamp_oil".to_string()))
        .copied()
        .unwrap_or_else(|| panic!("lamp_oil missing"))
}

fn flags_of(state: &State, id: &str) -> Vec<String> {
    assert!(state.actors().contains(&id.into()), "{id} missing");
    let id = id.into();
    let flags = state.actors().get::<component::Flag>(&id);
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
        id: "note_mira_door".to_string(),
        to: vec!["rowan".into()],
        place: Some("docks_shop".into()),
        fact_ids: Vec::new(),
        text: "Back at dusk. Oil on the shelf.".to_string(),
    }
}

/// Rowan is the one seated player.
fn rowan() -> Presence {
    Presence {
        players: 1,
        actors_present: vec!["rowan".into()],
    }
}

fn evening_note() {
    let pack = support::copy_docks("evening");
    play_pack_once(&pack).unwrap_or_else(|err| panic!("round: {err}"));
    let state = state_of(&pack);
    assert_eq!(lamp_oil(&state), 4);
    assert_eq!(place_of(&state, "mira"), "mira_rooms");
    assert_eq!(state.notes().len(), 1);
    assert_eq!(state.notes()[0].id, "note_mira_door");
    assert_eq!(state.notes()[0].place, Some("docks_shop".into()));
    let _ = fs::remove_dir_all(&pack);
}

fn morning_restock() {
    let pack = support::copy_docks("morning");
    {
        let opened = resume(&pack);
        let mut state = opened.state().clone();
        drop(opened);
        state.world_time = 600;
        state.notes_mut().leave(door_note()).unwrap();
        state
            .actors_mut()
            .get_mut::<component::Place>(&"rowan".into())
            .set("docks_street")
            .unwrap();
        write_save(&pack, &state);
    }
    assert_eq!(place_of(&state_of(&pack), "mira"), "mira_rooms");
    play_pack_once(&pack).unwrap_or_else(|err| panic!("round: {err}"));
    let state = state_of(&pack);
    assert_eq!(lamp_oil(&state), 8);
    assert_eq!(place_of(&state, "mira"), "mira_rooms");
    assert_eq!(state.notes().len(), 1);
    let _ = fs::remove_dir_all(&pack);
}

fn capped_restock() -> u32 {
    let pack = support::copy_docks("capped");
    {
        let opened = resume(&pack);
        let mut state = opened.state().clone();
        drop(opened);
        state.world_time = 600;
        state.notes_mut().leave(door_note()).unwrap();
        state
            .actors_mut()
            .get_mut::<component::Place>(&"rowan".into())
            .set("docks_street")
            .unwrap();
        let entry = ("docks_shop".into(), "lamp_oil".to_string());
        let stock = state.places().get::<component::Item>(&entry.0);
        let held = i64::from(stock.get(&entry.1).copied().unwrap_or(0));
        let mut places = state.places_mut();
        let mut shop = places.get_mut::<component::Item>(&entry.0);
        shop.shift("lamp_oil", 6 - held, None).unwrap();
        write_save(&pack, &state);
    }
    play_pack_once(&pack).unwrap_or_else(|err| panic!("round: {err}"));
    let oil = lamp_oil(&state_of(&pack));
    let _ = fs::remove_dir_all(&pack);
    oil
}

fn off_hours_hold() {
    let pack = support::copy_docks("off-hours");
    {
        let opened = resume(&pack);
        let mut state = opened.state().clone();
        drop(opened);
        state.notes_mut().leave(door_note()).unwrap();
        write_save(&pack, &state);
    }
    play_pack_once(&pack).unwrap_or_else(|err| panic!("round: {err}"));
    let state = state_of(&pack);
    assert_eq!(lamp_oil(&state), 4);
    assert_eq!(state.notes().len(), 1);
    let _ = fs::remove_dir_all(&pack);
}

fn player_in_the_shop() {
    let pack = support::copy_docks("present");
    {
        let opened = resume(&pack);
        let mut state = opened.state().clone();
        drop(opened);
        state.world_time = 600;
        state.notes_mut().leave(door_note()).unwrap();
        state
            .actors_mut()
            .get_mut::<component::Place>(&"mira".into())
            .set("docks_shop")
            .unwrap();
        write_save(&pack, &state);
    }
    let mut opened = resume(&pack);
    let wall = opened.state().last_round_at.clone();
    opened
        .play(&rowan(), &wall)
        .unwrap_or_else(|err| panic!("round: {err}"));
    let state = state_of(&pack);
    assert_eq!(lamp_oil(&state), 4);
    assert_eq!(state.notes().len(), 1);
    assert_eq!(act_of(&state, "mira").last_act_round, 0);
    let _ = fs::remove_dir_all(&pack);
}

fn queue_gm_intent(opened: &mut spookery_engine::World, id: &str, actor: &str, move_name: &str) {
    opened
        .queue_gm_intent(
            &spookery_engine::Presence::default(),
            spookery_protocol::QueueArgs {
                id: None,
                round_index: None,
                actor_id: Some(actor.to_string()),
                move_name: Some(move_name.to_string()),
                target: None,
                message_to: None,
                rationale_code: String::new(),
            },
            || Ok(id.to_string()),
        )
        .unwrap();
}

fn gm_restocks() {
    let pack = support::copy_docks("gm-restock");
    {
        let opened = resume(&pack);
        let mut state = opened.state().clone();
        drop(opened);
        state.world_time = 600;
        state.notes_mut().leave(door_note()).unwrap();
        state
            .actors_mut()
            .get_mut::<component::Place>(&"mira".into())
            .set("docks_shop")
            .unwrap();
        write_save(&pack, &state);
    }
    let mut opened = resume(&pack);
    let wall = opened.state().last_round_at.clone();
    queue_gm_intent(&mut opened, "gm-1", "mira", "restock_lamp_oil");
    opened.play(&rowan(), &wall).unwrap();
    assert_eq!(lamp_oil(opened.state()), 8);
    assert_eq!(opened.state().notes().len(), 1);
    let _ = fs::remove_dir_all(&pack);
}

fn claimed_note_sorts_first() {
    let pack = support::copy_docks("claimed");
    {
        let opened = resume(&pack);
        let mut state = opened.state().clone();
        drop(opened);
        state
            .actors_mut()
            .get_mut::<component::Place>(&"mira".into())
            .set("docks_shop")
            .unwrap();
        write_save(&pack, &state);
    }
    let mut opened = resume(&pack);
    let wall = opened.state().last_round_at.clone();
    for (id, actor, move_name) in [
        ("gm-1", "bram", "wait"),
        ("gm-2", "mira", "leave_door_note"),
    ] {
        queue_gm_intent(&mut opened, id, actor, move_name);
    }
    opened.play(&rowan(), &wall).unwrap();
    let log = round_value(&pack, "round-0001");
    let rules: Vec<&str> = log["extra_steps"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|step| step["rule"].as_str())
        .collect();
    assert_eq!(rules, vec!["cast:mira/leave_door_note", "verb:wait"]);
    let note = log["effects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|effect| effect["kind"] == "note")
        .unwrap_or_else(|| panic!("no door note: {log}"));
    assert_eq!(note["to"], serde_json::json!(["rowan"]));
    let _ = fs::remove_dir_all(&pack);
}

fn catch_up_to_cap() {
    catch_up_for(72, 12);
}

fn catch_up_for(hours: u64, rounds: u64) {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let pack = support::fresh_dir("catch_up");
    let _ = fs::remove_dir_all(&pack);
    let rules = manifest.join("../../packs/oneshot-docks/rules");
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
        &pack.join("world/places/gate.md"),
        "\
---
schema: place
id: gate
display_name: Gate
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
display_name: Rowan
---
Rowan.
",
    );
    for name in ["verbs.yaml", "checks.yaml", "meters.yaml", "budget.yaml"] {
        fs::copy(rules.join(name), {
            let dest = pack.join("rules").join(name);
            fs::create_dir_all(dest.parent().unwrap()).unwrap();
            dest
        })
        .unwrap_or_else(|err| panic!("copy {name}: {err}"));
    }
    let rev = spookery_engine::pack_rev(&pack).unwrap();
    let mut state = empty_save(load_canon(&pack).unwrap(), "ex", &rev);
    add_actor(&mut state, "rowan", component::ControlType::Pc, "gate", 1);
    write_text(&pack.join("state/save.yaml"), &emit_save(&state).unwrap());
    let place_before = fs::read(pack.join("world/places/gate.md")).unwrap();
    let cast_before = fs::read(pack.join("cast/rowan.md")).unwrap();
    let mut opened = resume(&pack);
    let wall = opened.state().last_round_at.clone();
    opened
        .catch_up(&wall, hours)
        .unwrap_or_else(|err| panic!("catch-up: {err}"));
    let saved = spookery_engine::World::peek(&pack).unwrap_or_else(|err| panic!("peek: {err}"));
    let state = saved.state();
    assert_eq!(state.state_rev, rounds, "{hours} hours");
    assert_eq!(state.round_index, rounds);
    assert_eq!(state.world_time, rounds * 60);
    assert_eq!(state.last_round_at, "2026-01-01T00:00:00Z");
    assert_eq!(list_round_keys(&pack).len() as u64, rounds);
    assert_eq!(
        fs::read(pack.join("world/places/gate.md")).unwrap(),
        place_before
    );
    assert_eq!(fs::read(pack.join("cast/rowan.md")).unwrap(), cast_before);
    let _ = fs::remove_dir_all(&pack);
}

fn a_busy_npc_skips() {
    let pack = support::copy_docks("busy");
    {
        let opened = resume(&pack);
        let mut state = opened.state().clone();
        drop(opened);
        state.world_time = 600;
        state.notes_mut().leave(door_note()).unwrap();
        let mut actors = state.actors_mut();
        actors
            .get_mut::<component::Act>(&"mira".into())
            .set_busy(u64::MAX, None)
            .unwrap();
        state
            .actors_mut()
            .get_mut::<component::Place>(&"rowan".into())
            .set("docks_street")
            .unwrap();
        write_save(&pack, &state);
    }
    play_pack_once(&pack).unwrap_or_else(|err| panic!("round: {err}"));
    let state = state_of(&pack);
    assert_eq!(lamp_oil(&state), 4);
    assert_eq!(act_of(&state, "mira").last_act_round, 0);
    assert_eq!(act_of(&state, "bram").last_act_round, 0);
    let _ = fs::remove_dir_all(&pack);
}

fn budget_caps() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let pack = support::fresh_dir("budget");
    let _ = fs::remove_dir_all(&pack);
    let rules = manifest.join("../../packs/oneshot-docks/rules");
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
        &pack.join("world/places/yard.md"),
        "\
---
schema: place
id: yard
display_name: Yard
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
display_name: {id}
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
    for name in ["verbs.yaml", "checks.yaml", "meters.yaml", "budget.yaml"] {
        let dest = pack.join("rules").join(name);
        fs::create_dir_all(dest.parent().unwrap()).unwrap();
        fs::copy(rules.join(name), &dest).unwrap();
    }
    let rev = spookery_engine::pack_rev(&pack).unwrap();
    let mut state = empty_save(load_canon(&pack).unwrap(), "crowd", &rev);
    for id in &npcs {
        add_actor(&mut state, id, component::ControlType::Npc, "yard", 1);
    }
    write_text(&pack.join("state/save.yaml"), &emit_save(&state).unwrap());
    play_pack_once(&pack).unwrap_or_else(|err| panic!("round: {err}"));
    let state = state_of(&pack);
    assert_eq!(state.round_index, 1);
    assert_eq!(
        state
            .actors()
            .control_type
            .values()
            .filter(|control_type| !control_type.is_pc())
            .count(),
        50
    );
    let acted = state
        .actors()
        .acts
        .values()
        .filter(|act| act.last_act_round == 1)
        .count();
    let applied = round_value(&pack, "round-0001")["extra_steps"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|step| step["rule"].is_string())
        .count();
    assert!((1..=8).contains(&acted), "acted {acted}");
    assert!((1..=12).contains(&applied), "applied {applied}");
    let _ = fs::remove_dir_all(&pack);
}

fn a_zero_budget_strips_fact_bodies() {
    let pack = support::copy_docks("scope");
    let before = snapshot(&pack);
    let opened = spookery_engine::World::open(&pack).unwrap();
    let call = |as_viewer: &str, budget: usize| {
        opened.perceive_trimmed(&Presence::default(), as_viewer, budget)
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
    let pack = support::copy_docks("strip");
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
    let mut state = State::new(save, Arc::new(canon));
    state
        .actors_mut()
        .get_mut::<component::Place>(&"mira".into())
        .set("docks_shop")
        .unwrap();
    write_save(pack, &state);
}

fn play_mark(pack: &Path) -> State {
    let mut opened = resume(pack);
    let wall = opened.state().last_round_at.clone();
    opened
        .queue_player_move(&rowan(), &"rowan".into(), "mark", Some("mira"))
        .unwrap();
    let played = opened.commit_then_fast_forward(&rowan(), &wall).unwrap();
    assert!(!played.is_empty(), "mark mira did not commit");
    opened.state().clone()
}

fn effects_run_on_both_outcomes_and_lists_run_on_one() {
    for (label, difficulty, success) in [("effects-pass", 0, true), ("effects-fail", 14, false)] {
        let pack = support::copy_docks(label);
        let mut moves = fs::read_to_string(pack.join("rules/verbs.yaml")).unwrap();
        moves.push_str(&format!(
            "\n  - id: mark\n    when:\n      verb: mark\n      target: actor\n    roll:\n      check: grit\n      threshold: {difficulty}\n    do:\n      - flag:\n          of: target\n          key: marked\n          value: true\n",
        ));
        fs::write(pack.join("rules/verbs.yaml"), moves).unwrap();
        patch_mira_flags_and_place(&pack, "    - marked\n");
        stamp_pack_rev(&pack);
        let state = play_mark(&pack);
        assert_eq!(state.world_time, 1085);
        assert_eq!(
            flags_of(&state, "mira"),
            vec!["marked".to_string()],
            "{label} success {success}"
        );
        let round = round_value(&pack, "round-0001");
        assert!(round["extra_steps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|name| name == "check_rolled"));
        assert!(
            !spookery_engine::testing::journal::read_record_line(&pack, "round-0001")
                .unwrap_or_else(|err| panic!("{}", err.detail))
                .contains("faces")
        );
        let _ = fs::remove_dir_all(&pack);
    }

    for (label, difficulty, flag) in [("lists-pass", 0, "marked"), ("lists-fail", 14, "missed")] {
        let pack = support::copy_docks(label);
        let mut moves = fs::read_to_string(pack.join("rules/verbs.yaml")).unwrap();
        moves.push_str(&format!(
            "\n  - id: mark\n    when:\n      verb: mark\n      target: actor\n    roll:\n      check: grit\n      threshold: {difficulty}\n    success:\n      - flag:\n          of: target\n          key: marked\n          value: true\n    miss:\n      - flag:\n          of: target\n          key: missed\n          value: true\n",
        ));
        fs::write(pack.join("rules/verbs.yaml"), moves).unwrap();
        patch_mira_flags_and_place(&pack, "    - marked\n    - missed\n");
        stamp_pack_rev(&pack);
        let state = play_mark(&pack);
        assert_eq!(flags_of(&state, "mira"), vec![flag.to_string()], "{label}");
        assert_eq!(state.world_time, 1085);
        let _ = fs::remove_dir_all(&pack);
    }
}

fn roll_with_stat(reports: &[MoveReport], stat: i64) -> spookery_protocol::Roll {
    reports
        .iter()
        .find_map(|report| match report {
            MoveReport::Rolled { roll: out, .. } if out.stat == stat => Some(out.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no stat {stat} in {reports:?}"))
}

fn opposing_total_reused() {
    for opposed in [false, true] {
        let label = if opposed { "opposed" } else { "open-check" };
        let pack = support::copy_docks(label);
        let mut moves = fs::read_to_string(pack.join("rules/verbs.yaml")).unwrap();
        moves.push_str(
            "\n  - id: mark\n    when:\n      verb: mark\n      target: actor\n    roll:\n      check: grit\n      threshold: 0\n    success:\n      - flag:\n          of: target\n          key: marked\n          value: true\n    miss:\n      - flag:\n          of: target\n          key: missed\n          value: true\n  - id: defend\n    when:\n      verb: defend\n    roll:\n      check: grit\n      threshold: 0\n    success:\n      - flag:\n          of: self\n          key: guarded\n          value: true\n",
        );
        fs::write(pack.join("rules/verbs.yaml"), moves).unwrap();
        insert_nell(&pack);
        stamp_pack_rev(&pack);
        let mut opened = resume(&pack);
        if opposed {
            queue_gm_intent(&mut opened, "gm-1", "nell", "defend");
        }
        let wall = opened.state().last_round_at.clone();
        opened
            .queue_player_move(&rowan(), &"rowan".into(), "mark", Some("nell"))
            .unwrap();
        assert_eq!(opened.state().world_time, 1080);
        let outputs = opened.commit_then_fast_forward(&rowan(), &wall).unwrap();
        let reports = outputs.into_iter().next().unwrap().reports;
        let rowan = roll_with_stat(&reports, 1);
        assert_eq!(opened.state().world_time, 1085);
        let round = spookery_engine::testing::journal::read_record_line(&pack, "round-0001")
            .unwrap_or_else(|err| panic!("{}", err.detail));
        assert!(!round.contains("faces"));
        if opposed {
            let nell = roll_with_stat(&reports, 100);
            assert_eq!(nell.target, 0);
            assert_eq!(nell.outcome, Outcome::Success);
            assert!(nell.total >= 102, "{nell:?}");
            assert_eq!(rowan.target, nell.total);
            assert_eq!(rowan.outcome, Outcome::Miss);
            let mut flags = flags_of(opened.state(), "nell");
            flags.sort();
            assert_eq!(flags, vec!["guarded".to_string(), "missed".to_string()]);
        } else {
            assert_eq!(rowan.target, 0);
            assert_eq!(rowan.outcome, Outcome::Success);
            assert_eq!(flags_of(opened.state(), "nell"), vec!["marked".to_string()]);
        }
        let _ = fs::remove_dir_all(&pack);
    }
}

fn a_held_knife_still_runs_the_opposed_row() {
    let held = support::copy_docks("item-held");
    let mut moves = fs::read_to_string(held.join("rules/verbs.yaml")).unwrap();
    moves.push_str(
        "\n  - id: mark\n    when:\n      verb: mark\n      target: actor\n    if:\n      - has_item:\n          subject: self\n          id: knife\n    roll:\n      check: grit\n      threshold: 0\n    success:\n      - flag:\n          of: target\n          key: marked\n          value: true\n    miss:\n      - flag:\n          of: target\n          key: missed\n          value: true\n  - id: defend\n    when:\n      verb: defend\n    roll:\n      check: grit\n      threshold: 0\n    success:\n      - flag:\n          of: self\n          key: guarded\n          value: true\n",
    );
    fs::write(held.join("rules/verbs.yaml"), moves).unwrap();
    let knife = held.join("world/items/knife.md");
    fs::create_dir_all(knife.parent().unwrap()).unwrap();
    fs::write(
        &knife,
        "---\nschema: item\nid: knife\ndisplay_name: Knife\ntags: []\nrules: []\n---\n\nA knife.\n",
    )
    .unwrap();
    insert_nell(&held);
    let now_path = held.join("state/save.yaml");
    let text = fs::read_to_string(&now_path).unwrap();
    let next = text.replacen(
        "  items: {}\n  meters:\n",
        "  items:\n    rowan:\n      knife: 1\n  meters:\n",
        1,
    );
    assert_ne!(next, text);
    fs::write(&now_path, next).unwrap();
    stamp_pack_rev(&held);
    let mut opened = resume(&held);
    queue_gm_intent(&mut opened, "gm-1", "nell", "defend");
    let wall = opened.state().last_round_at.clone();
    opened
        .queue_player_move(&rowan(), &"rowan".into(), "mark", Some("nell"))
        .unwrap();
    let before = opened.state().world_time;
    let outputs = opened.commit_then_fast_forward(&rowan(), &wall).unwrap();
    assert_ne!(opened.state().world_time, before);
    let rowan = roll_with_stat(&outputs[0].reports, 1);
    assert_eq!(rowan.outcome, Outcome::Miss);
    assert!(
        !spookery_engine::testing::journal::read_record_line(&held, "round-0001")
            .unwrap_or_else(|err| panic!("{}", err.detail))
            .contains("faces")
    );
    let _ = fs::remove_dir_all(&held);
}

fn rowans_debt(pack: &Path) -> i64 {
    let rowan = entity::Id::Actor("rowan".into());
    state_of(pack)
        .entities()
        .get::<component::Meter>(&rowan)
        .value("assassination_debt")
        .unwrap_or_else(|err| panic!("assassination_debt: {err}"))
}

#[given(expr = "rowan's {string} meter is {int}")]
fn rowans_meter_is(world: &mut World, key: String, value: i64) {
    assert_eq!(key, "assassination_debt");
    let pack = support::copy_docks("debt");
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
        state_of(world.pack()).round_index,
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
    let pack = support::copy_docks("crash");
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
    assert_eq!(host.state().state_rev, before.state_rev);
    assert_eq!(lamp_oil(host.state()), lamp_oil(&before));
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

#[when(expr = "an opposing total is in the queue")]
fn an_opposing_total_is_in_the_queue(_world: &mut World) {
    opposing_total_reused();
}

#[when(expr = "Rowan marks Nell while holding the knife")]
fn rowan_marks_nell_while_holding_the_knife(_world: &mut World) {
    a_held_knife_still_runs_the_opposed_row();
}

/// A save with no actors on `canon`, for world `world_id` at pack revision `rev`.
fn empty_save(canon: Canon, world_id: &str, rev: &str) -> State {
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
notes: []
"
    );
    let save = spookery_engine::parse_save(&text).unwrap_or_else(|err| panic!("save: {err:?}"));
    State::new(save, Arc::new(canon))
}
