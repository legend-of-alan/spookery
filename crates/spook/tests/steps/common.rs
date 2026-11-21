//! Steps used by more than one cucumber program.

use spookery_engine::testing::{place_of, world_of};
use std::fs;
use std::io::Write;
use std::path::Path;

use cucumber::{given, then, when};
use spookery_engine::{component, ecs};

use super::table;
use crate::support::{self, copy_pack};
use crate::World;

pub(crate) const EVENING: u64 = 1290;

pub(crate) use crate::support::edit_save;

/// Rowan holds brass_key, and the stair door is open, as a round that ended with him in the
/// shop would have left it.
/// The flags `actor` holds, in name order.
pub(crate) fn flags_of(world: &ecs::World, actor: &str) -> Vec<String> {
    let actor = actor.into();
    let flags = world.actors().get::<component::Flag>(&actor);
    flags.iter().map(|(flag, ())| flag.to_string()).collect()
}

pub(crate) fn give_key(now: &mut ecs::World) {
    now.actors_mut()
        .get_mut::<component::Item>(&"rowan".into())
        .shift("brass_key", 1, None)
        .unwrap();
    let stairs = spookery_engine::ExitId {
        place: "docks_shop".into(),
        exit: "stairs".to_string(),
    };
    now.places_mut().set_exit_open(&stairs, true).unwrap();
}

pub(crate) fn add_trapdoor(pack: &Path) {
    let path = pack.join("world/places/docks_shop.md");
    let text = fs::read_to_string(&path).unwrap();
    let next = text.replace(
        "    text: Oil is on the shelf.\n",
        "    text: Oil is on the shelf.\n  - id: fact_trapdoor\n    text: A trapdoor is under the rug.\n    hidden: true\n",
    );
    let next = next.replace(
        "    locked_text: The stair door is locked.\n",
        "    locked_text: The stair door is locked.\n  - to: cellar\n    id: trapdoor\n    hidden: true\n    revealed_by: fact_trapdoor\n",
    );
    assert_ne!(text, next);
    fs::write(&path, next).unwrap();
    fs::write(
        pack.join("world/places/cellar.md"),
        "---\nschema: place\nid: cellar\nname: Cellar\nfacts: []\nexits: []\n---\nUnder the shop.\n",
    )
    .unwrap();
}

pub(crate) async fn run_check_json(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::run(
        world,
        &["check", "--json", &crate::support::pack_arg(&pack)],
    )
    .await;
}

/// Appends `yaml`, one or more verb rules, to the pack's `rules/verbs.yaml`.
pub(crate) fn add_verb(pack: &Path, yaml: &str) {
    let path = pack.join("rules/verbs.yaml");
    let mut text = fs::read_to_string(&path).unwrap();
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(yaml);
    fs::write(path, text).unwrap();
}

pub(crate) fn replace_in_file(path: &Path, from: &str, to: &str) {
    let text = fs::read_to_string(path).unwrap();
    assert!(text.contains(from), "missing {from} in {}", path.display());
    fs::write(path, text.replacen(from, to, 1)).unwrap();
}

/// Adds `actor` to the save as a pc in the shop, whether or not the cast has them.
fn add_saved_actor(pack: &Path, actor: &str) {
    let acts = "last_act_round: 0\n      idle_acts_this_window: 0\n      live_acts_this_window: 0";
    for (after, row) in [
        ("    aldric: npc\n", format!("    {actor}: pc\n")),
        (
            "    aldric: docks_shop\n",
            format!("    {actor}: docks_shop\n"),
        ),
        ("  acts:\n", format!("    {actor}:\n      {acts}\n")),
    ] {
        let path = pack.join("state/save.yaml");
        replace_in_file(&path, after, &format!("{after}{row}"));
    }
}

/// Writes `text` to the pack file at `file`, making its folder when the pack has none.
fn write_file(pack: &Path, file: &str, text: &str) {
    let path = pack.join(file);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// Writes `story/tide.yaml`: one rule, `mark`, that watches the world's tide turn, with
/// `fields` written after its `when`.
pub(crate) fn write_tide_story(pack: &Path, fields: &str) {
    let when = "when:\n      change:\n        flag: tide_turned\n        becomes: true";
    let text = format!("schema: story\nid: tide\nrules:\n  - id: mark\n    {when}\n{fields}");
    write_file(pack, "story/tide.yaml", &text);
}

/// The body of a verb rule that sets the world flag `tide_turned`, which makes `mark` in
/// `story/tide.yaml` a world change rule.
const TURN_TIDE: &str = "    do:\n      - flag: { of: world, key: tide_turned, value: true }\n";

/// Writes `rules/modifiers.yaml` with these rows.
fn write_modifiers(pack: &Path, rows: &str) {
    let text = format!("schema: modifiers\nmodifiers:\n{rows}");
    write_file(pack, "rules/modifiers.yaml", &text);
}

/// Writes `world/meters.yaml` with one meter, `dawn`.
fn write_world_dawn(pack: &Path) {
    let text = format!(
        "schema: meters\nmeters:\n{}",
        format_meter_row("dawn", None)
    );
    write_file(pack, "world/meters.yaml", &text);
}

/// A meters list row for meter `id`, from 0 to 6 at base 0, with its `hidden` key when it
/// has one.
pub(crate) fn format_meter_row(id: &str, hidden: Option<bool>) -> String {
    let hidden = hidden.map_or(String::new(), |hidden| format!("    hidden: {hidden}\n"));
    format!("  - id: {id}\n    min: 0\n    base_cap: 6\n    max: 6\n    base: 0\n{hidden}")
}

/// Appends verb rule `verb`, for the verb of the same id, with `body` after its `when`.
fn add_verb_rule(pack: &Path, verb: &str, body: &str) {
    add_verb(
        pack,
        &format!("\n  - id: {verb}\n    when:\n      verb: {verb}\n{body}"),
    );
}

/// Gives the `ending` story a `meters:` list of this one meter.
fn declare_ending_meter(pack: &Path, meter: &str) {
    let meters = format!("id: ending\nmeters:\n{}", format_meter_row(meter, None));
    replace_in_file(&pack.join("story/ending.yaml"), "id: ending\n", &meters);
}

fn break_pack(pack: &Path, fault: &str) {
    match fault {
        "missing when" => replace_in_file(
            &pack.join("rules/verbs.yaml"),
            "  - id: exert\n    when:\n      verb: exert\n",
            "  - id: exert\n",
        ),
        "unknown field" => replace_in_file(
            &pack.join("rules/verbs.yaml"),
            "  - id: exert\n    when:\n",
            "  - id: exert\n    audiance: player\n    when:\n",
        ),
        "duplicate move" => replace_in_file(
            &pack.join("rules/verbs.yaml"),
            "  - id: wait\n",
            "  - id: exert\n",
        ),
        "unknown format tag" => replace_in_file(
            &pack.join("rules/verbs.yaml"),
            "schema: verbs\n",
            "schema: bogus\n",
        ),
        "missing meters" => {
            fs::remove_file(pack.join("rules/meters.yaml")).unwrap();
        }
        "meter min above base_cap" => replace_in_file(
            &pack.join("rules/meters.yaml"),
            "    min: 0
    base_cap: 100
",
            "    min: 200
    base_cap: 100
",
        ),
        "meter base_cap above max" => replace_in_file(
            &pack.join("rules/meters.yaml"),
            "    base_cap: 5
    max: 5
",
            "    base_cap: 5
    max: 4
",
        ),
        "meter base outside its range" => replace_in_file(
            &pack.join("rules/meters.yaml"),
            "    base: 3
",
            "    base: 9
",
        ),
        "pack_rev mismatch" => {
            let path = pack.join("state/save.yaml");
            let text = fs::read_to_string(&path).unwrap();
            let text = text.replacen("pack_rev: blake2s:", "pack_rev: blake2s:0", 1);
            fs::write(path, text).unwrap();
        }
        "an actor the cast lacks" => add_saved_actor(pack, "ghost"),
        "a control type with two keys" => replace_in_file(
            &pack.join("state/save.yaml"),
            "    aldric: npc\n",
            "    aldric: { npc: null, pc: null }\n",
        ),
        "exit to an unknown place" => replace_in_file(
            &pack.join("world/places/docks_shop.md"),
            "  - to: docks_street\n",
            "  - to: nowhere\n",
        ),
        "duplicate exit" => replace_in_file(
            &pack.join("world/places/docks_shop.md"),
            "    id: docks_street\n",
            "    id: docks_street\n  - to: docks_street\n    id: docks_street\n",
        ),
        "duplicate place fact" => replace_in_file(
            &pack.join("world/places/docks_shop.md"),
            "    text: Oil is on the shelf.\n",
            "    text: \"She said: oil.\"\n  - id: fact_oil_on_shelf\n    text: Oil again.\n",
        ),
        "cast facts" => replace_in_file(
            &pack.join("cast/pc-rowan.md"),
            "name: Rowan\n",
            "name: Rowan\nfacts: []\n",
        ),
        "a target alias" => replace_in_file(
            &pack.join("rules/verbs.yaml"),
            "      target: item\n",
            "      target: item_here\n",
        ),
        "a verb named look" => replace_in_file(
            &pack.join("rules/verbs.yaml"),
            "      verb: wait\n",
            "      verb: look\n",
        ),
        "a verb named say" => replace_in_file(
            &pack.join("rules/verbs.yaml"),
            "      verb: wait\n",
            "      verb: say\n",
        ),
        "a cast role named kind" => replace_in_file(
            &pack.join("cast/pc-rowan.md"),
            "control_type: pc_template\n",
            "kind: pc_template\n",
        ),
        "an empty tags key" => replace_in_file(
            &pack.join("world/items/latch.md"),
            "tags: []\n",
            "tags:\n",
        ),
        "an empty modifiers key" => replace_in_file(
            &pack.join("state/save.yaml"),
            "  modifiers: {}\n",
            "  modifiers:\n",
        ),
        "flags under idle" => replace_in_file(
            &pack.join("cast/pc-rowan.md"),
            "  eligible: false\n",
            "  eligible: false\n  flags: []\n",
        ),
        "idle maybe" => replace_in_file(
            &pack.join("rules/budget.yaml"),
            "schema: budget\n",
            "schema: budget\nidle: maybe\n",
        ),
        "zero round interval" => replace_in_file(
            &pack.join("rules/budget.yaml"),
            "round_every_real_seconds: 300\n",
            "round_every_real_seconds: 0\n",
        ),
        "zero idle ticks" => replace_in_file(
            &pack.join("rules/budget.yaml"),
            "idle_ticks_per_round: 60\n",
            "idle_ticks_per_round: 0\n",
        ),
        "zero idle window" => replace_in_file(
            &pack.join("rules/budget.yaml"),
            "idle_act_window_ticks: 1440\n",
            "idle_act_window_ticks: 0\n",
        ),
        "a zero-tick cycle" => replace_in_file(
            &pack.join("rules/budget.yaml"),
            "  day: 1440\n",
            "  day: 0\n",
        ),
        "a cycle past its length" => replace_in_file(
            &pack.join("cast/mira.md"),
            "          to: 960\n",
            "          to: 1440\n",
        ),
        "an unknown cycle" => replace_in_file(
            &pack.join("cast/mira.md"),
            "          id: day\n",
            "          id: tide\n",
        ),
        "world as an actor" => replace_in_file(
            &pack.join("world/places/gate_yard.md"),
            "      - has_item:\n          subject: self\n          id: chock\n",
            "      - at:\n          subject: world\n          place: gate_yard\n",
        ),
        "target on a move with none" => replace_in_file(
            &pack.join("rules/verbs.yaml"),
            "      verb: wait\n    do: []\n",
            "      verb: wait\n    do:\n      - meter: { of: target, key: hp, add: -1 }\n",
        ),
        "owner target on an item move" => replace_in_file(
            &pack.join("rules/verbs.yaml"),
            "      verb: drop\n      target: item\n    do:\n",
            "      verb: drop\n      target: item\n    do:\n      - meter: { of: target, key: hp, add: -1 }\n",
        ),
        "id: target with no target" => replace_in_file(
            &pack.join("rules/verbs.yaml"),
            "      verb: wait\n    do: []\n",
            "      verb: wait\n    do:\n      - item: { of: self, id: target, add: 1 }\n",
        ),
        "a save by target with none" => replace_in_file(
            &pack.join("rules/verbs.yaml"),
            "      verb: wait\n    do: []\n",
            "      verb: wait\n    do:\n      - check: { by: target, total: 1d20, success_at: 12 }\n",
        ),
        "target on an exit" => replace_in_file(
            &pack.join("world/places/gate_yard.md"),
            "          of: world\n          key: passage\n",
            "          of: target\n          key: passage\n",
        ),
        "a meter on the wrong actor" => replace_in_file(
            &pack.join("cast/bram.md"),
            "  eligible: true\n",
            "  eligible: true\nrules:\n  - id: tally_shop_day\n    when: habit\n    do:\n      - meter: { of: self, key: shop_day, add: 1 }\n",
        ),
        "a cast meter that is global" => {
            let meters = format!("meters:\n{}", format_meter_row("vigor", None));
            replace_in_file(&pack.join("cast/mira.md"), "meters:\n", &meters);
        }
        "a meter its actor lacks" => {
            write_world_dawn(pack);
            replace_in_file(
                &pack.join("cast/mira.md"),
                "rules:\n",
                "rules:\n  - id: bump_dawn\n    when: habit\n    do:\n      - meter: { of: self, key: dawn, add: 1 }\n",
            );
        }
        "self in a world change rule" => {
            add_verb_rule(pack, "turn_tide", TURN_TIDE);
            let fields = "    do:\n      - flag: { of: self, key: saw_tide, value: true }\n";
            write_tide_story(pack, fields);
        }
        "a check in a world change rule" => {
            add_verb_rule(pack, "turn_tide", TURN_TIDE);
            write_tide_story(pack, "    check:\n      total: 2d6\n      success_at: 7\n");
        }
        "a narrate with no to" => add_verb_rule(
            pack,
            "ring",
            "    do:\n      - narrate: { text: The bell rings. }\n",
        ),
        "a room narrate of a known fact" => write_file(
            pack,
            "story/secret.yaml",
            "schema: story\nid: secret\nrules:\n  - id: tell\n    when:\n      change:\n        knows: fact_ledger\n    do:\n      - narrate: { to: here, text: A secret moved. }\n",
        ),
        "max_stacks 0" => write_modifiers(
            pack,
            "  - id: fatigue\n    meter: vigor\n    max_stacks: 0\n",
        ),
        "a modifier of no meter" => write_modifiers(pack, "  - id: chill\n    meter: warmth\n"),
        "a modifier for 0 ticks" => {
            write_modifiers(pack, "  - id: numb\n    meter: vigor\n");
            add_verb_rule(
                pack,
                "numbing",
                "    do:\n      - modifier: { to: self, id: numb, add: -1, ticks: 0 }\n",
            );
        }
        "a modifier listed twice" => write_modifiers(
            pack,
            "  - id: fatigue\n    meter: vigor\n    max_stacks: 4\n  - id: fatigue\n    meter: vigor\n",
        ),
        "an undefined modifier" => add_verb_rule(
            pack,
            "scorch",
            "    do:\n      - modifier: { to: self, id: scorch, add: -1, ticks: 10 }\n",
        ),
        "an undeclared story flag" => {
            write_file(pack, "story/siege.yaml", "schema: story\nid: siege\nrules: []\n");
            add_verb_rule(
                pack,
                "set_mark",
                "    do:\n      - flag: { of: \"story:siege\", key: walls_breached, value: true }\n",
            );
        }
        "a story meter the world declares" => {
            write_world_dawn(pack);
            declare_ending_meter(pack, "dawn");
        }
        "a story meter a cast declares" => declare_ending_meter(pack, "shop_day"),
        "a story meter on the wrong story" => {
            declare_ending_meter(pack, "dread");
            write_file(
                pack,
                "story/prologue.yaml",
                "schema: story\nid: prologue\nrules:\n  - id: on_oil\n    when:\n      change:\n        item: lamp_oil\n        gained: true\n    do:\n      - meter: { of: \"story:prologue\", key: dread, add: 1 }\n",
            );
        }
        "an undeclared place meter" => add_verb_rule(
            pack,
            "bump",
            "    do:\n      - meter: { of: \"place:docks_street\", key: light, add: 1 }\n",
        ),
        "a repeated story id" => {
            write_file(pack, "story/later.yaml", "schema: story\nid: ending\nrules: []\n");
        }
        "an empty story id" => {
            replace_in_file(&pack.join("story/ending.yaml"), "id: ending\n", "id: \"\"\n");
        }
        "a walker on a missing exit" => replace_in_file(
            &pack.join("state/save.yaml"),
            "    rowan: docks_shop\n",
            "    rowan: exit:docks_shop.ladder\n",
        ),
        "a verb named cancel" => replace_in_file(
            &pack.join("rules/verbs.yaml"),
            "      verb: wait\n",
            "      verb: cancel\n",
        ),
        "a saved item with no definition" => replace_in_file(
            &pack.join("state/save.yaml"),
            "      tonic: 1\n",
            "      tonic: 1\n      ghost: 1\n",
        ),
        "partial_at without partial" => add_verb_rule(
            pack,
            "press",
            "    check:\n      total: 2d6 + self.vigor\n      success_at: 10\n      partial_at: 7\n    do:\n      - flag: { of: self, key: touched, value: true }\n",
        ),
        "partial without partial_at" => add_verb_rule(
            pack,
            "press",
            "    check:\n      total: 2d6 + self.vigor\n      success_at: 10\n    partial:\n      - flag: { of: self, key: halfway, value: true }\n",
        ),
        "partial_at above success_at" => add_verb_rule(
            pack,
            "press",
            "    check:\n      total: 2d6 + self.vigor\n      success_at: 10\n      partial_at: 10\n    success:\n      - flag: { of: self, key: touched, value: true }\n    partial:\n      - flag: { of: self, key: halfway, value: true }\n",
        ),
        "a nested partial_at without partial" => add_verb_rule(
            pack,
            "press",
            "    do:\n      - check:\n          total: 2d6 + self.vigor\n          success_at: 10\n          partial_at: 7\n          success:\n            - flag: { of: self, key: touched, value: true }\n",
        ),
        "partial without a check" => add_verb_rule(
            pack,
            "press",
            "    partial:\n      - flag: { of: self, key: halfway, value: true }\n",
        ),
        "a cast id world" => write_file(
            pack,
            "cast/world.md",
            "---\nschema: cast\nid: world\ncontrol_type: npc\nname: World\nidle:\n  eligible: false\n---\nReserved.\n",
        ),
        "a saved actor world" => add_saved_actor(pack, "world"),
        "a reveal of a missing fact" => add_verb_rule(
            pack,
            "bad",
            "    do:\n      - reveal: { to: self, fact: missing_fact }\n",
        ),
        other => panic!("unknown fault {other}"),
    }
}

/// Hosts the pack and runs play on it with `flags`, picking `pc`.
pub(crate) async fn join_play(world: &mut World, pc: &str, flags: &[&str]) {
    let pack = table::pack(world).to_path_buf();
    table::listen(world, &pack).await;
    let addr = table::game_addr(world);
    world.play = Some(spawn_play(&addr, pc, flags));
    let needle = if flags.contains(&"--raw") {
        "\"kind\":\"view\""
    } else {
        "You are in"
    };
    wait_play(world, needle).await;
}

/// Starts `spook play` on `addr` with `flags`, and types `pc` at its picker the way a player
/// does.
pub(crate) fn spawn_play(addr: &str, pc: &str, flags: &[&str]) -> support::Process {
    let mut args = vec!["play", "--addr", addr];
    args.extend_from_slice(flags);
    let mut play = support::spawn(&args);
    let stdin = play
        .stdin
        .as_mut()
        .unwrap_or_else(|| panic!("play stdin is closed"));
    writeln!(stdin, "{pc}").unwrap_or_else(|err| panic!("pick {pc}: {err}"));
    stdin.flush().unwrap_or_else(|err| panic!("flush: {err}"));
    play
}

pub(crate) async fn wait_play(world: &mut World, needle: &str) {
    wait_play_until(world, |stdout| stdout.contains(needle), needle).await;
}

/// Waits until play's stdout satisfies `ready`, and keeps it in `world.stdout`.
pub(crate) async fn wait_play_until(world: &mut World, ready: impl Fn(&str) -> bool, label: &str) {
    let play = world.play.as_mut().unwrap_or_else(|| panic!("no play"));
    world.stdout = wait_proc_until(play, ready, label).await;
}

/// Waits until `proc`'s stdout satisfies `ready`, and returns it. Panics if the
/// process exits first.
pub(crate) async fn wait_proc_until(
    proc: &mut support::Process,
    ready: impl Fn(&str) -> bool,
    label: &str,
) -> String {
    table::poll_until(label, async || {
        let stdout = support::text_of(&proc.stdout);
        if ready(&stdout) {
            return Ok(stdout);
        }
        let stderr = support::text_of(&proc.stderr);
        if let Some(status) = proc
            .child
            .try_wait()
            .unwrap_or_else(|err| panic!("try_wait: {err}"))
        {
            panic!("exited {status:?} before {label}\nstdout:\n{stdout}\nstderr:\n{stderr}");
        }
        Err(format!("stdout:\n{stdout}\nstderr:\n{stderr}"))
    })
    .await
}

pub(crate) fn capture_play_stdout(world: &mut World) -> String {
    let play = world.play.as_ref().unwrap_or_else(|| panic!("no play"));
    let text = support::text_of(&play.stdout);
    world.stdout = text.clone();
    text
}

/// Put `actor` on the exit `<place>.<exit>`, walking it until tick `until`.
pub(crate) fn set_busy(now: &mut ecs::World, actor: &str, until: u64, exit: &str) {
    let (mut actors, actor) = (now.actors_mut(), actor.into());
    actors
        .get_mut::<component::Place>(&actor)
        .set(&format!("exit:{exit}"))
        .unwrap();
    actors
        .get_mut::<component::Act>(&actor)
        .set_busy(until, Some(walk(exit)))
        .unwrap();
}

/// A walk through the exit `<place>.<exit>` by the travel verb.
pub(crate) fn walk(exit: &str) -> spookery_engine::testing::PendingAction {
    let (_, exit_id) = exit
        .split_once('.')
        .unwrap_or_else(|| panic!("no exit in {exit}"));
    spookery_engine::testing::PendingAction {
        verb: "travel".to_string(),
        source: spookery_engine::Source::Player,
        target: spookery_engine::Target::Exit(exit_id.to_string()),
        rule: "verb:travel".to_string(),
    }
}

#[given(expr = "a docks table")]
pub(crate) fn a_docks_table(world: &mut World) {
    let pack = crate::support::copy_pack("docks", "status");
    world.before = table::read_files(&pack);
    world.pack = Some(pack);
}

#[then(expr = "the pack bytes are unchanged")]
fn the_pack_bytes_are_unchanged(world: &mut World) {
    let pack = world
        .pack
        .as_ref()
        .unwrap_or_else(|| panic!("no docks table"));
    assert_eq!(
        table::read_files(pack),
        world.before,
        "the command wrote the pack"
    );
}

#[given(expr = "a docks table with a seated GM")]
async fn a_docks_table_with_a_seated_gm(world: &mut World) {
    let pack = copy_pack("docks", "gm-seat");
    let opened = fs::read(pack.join("state/save.yaml"))
        .unwrap_or_else(|err| panic!("read save.yaml: {err}"));
    table::listen(world, &pack).await;
    table::seat_gm(world).await;
    table::live_mut(world).opened_save = opened;
    world.pack = Some(pack);
}

#[given(expr = "a docks table with a seated player")]
async fn a_docks_table_with_a_seated_player(world: &mut World) {
    let pack = copy_pack("docks", "speech");
    table::listen(world, &pack).await;
    table::seat_rowan(world, &pack).await;
    world.pack = Some(pack);
}

#[when(expr = "the operator runs spook check")]
async fn the_operator_runs_spook_check(world: &mut World) {
    let pack = table::pack(world);
    let output = support::run_spook(&["check", &crate::support::pack_arg(pack)]).await;
    world.code = output.status.code();
    world.stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    world.stderr = String::from_utf8_lossy(&output.stderr).into_owned();
}

#[when(expr = "the operator runs spook check --json")]
async fn the_operator_runs_spook_check_json(world: &mut World) {
    run_check_json(world).await;
}

#[when(expr = "the copy is broken by {string}")]
fn the_copy_is_broken_by(world: &mut World, fault: String) {
    let pack = table::pack(world).to_path_buf();
    break_pack(&pack, &fault);
    world.before = table::read_files(&pack);
}

#[then(expr = "the check exits {int}")]
fn the_check_exits(world: &mut World, code: i32) {
    assert_eq!(world.code, Some(code), "{}\n{}", world.stdout, world.stderr);
}

#[then(expr = "stdout says {string}")]
fn stdout_says(world: &mut World, text: String) {
    assert_eq!(world.stdout.trim(), text, "{}", world.stdout);
}

#[then(expr = "the report says {string}")]
fn the_report_says(world: &mut World, text: String) {
    assert!(
        world.stderr.contains(&text),
        "missing {text}\n{}",
        world.stderr
    );
}

#[when(expr = "play joins the table on a free loopback address")]
async fn play_joins_the_table_on_a_free_loopback_address(world: &mut World) {
    join_play(world, "rowan", &[]).await;
}

#[when(expr = "play joins the table as {string}")]
async fn play_joins_the_table_as(world: &mut World, pick: String) {
    join_play(world, &pick, &[]).await;
}

#[when(expr = "play types {string}")]
fn play_types(world: &mut World, line: String) {
    write_play(world, &line);
}

pub(crate) fn write_play(world: &mut World, line: &str) {
    let play = world.play.as_mut().unwrap_or_else(|| panic!("no play"));
    let stdin = play
        .stdin
        .as_mut()
        .unwrap_or_else(|| panic!("play stdin is closed"));
    writeln!(stdin, "{line}").unwrap_or_else(|err| panic!("type: {err}"));
    stdin.flush().unwrap_or_else(|err| panic!("flush: {err}"));
}

#[then(expr = "play shows {string}")]
async fn play_shows(world: &mut World, text: String) {
    wait_play(world, &text).await;
}

#[then(expr = "play shows no protocol JSON")]
fn play_shows_no_protocol_json(world: &mut World) {
    let stdout = capture_play_stdout(world);
    assert!(!stdout.contains('{'), "{stdout}");
    assert!(!stdout.contains("\"kind\""), "{stdout}");
}

#[then(expr = "the command exits {int}")]
fn the_command_exits(world: &mut World, code: i32) {
    assert_eq!(
        world.code,
        Some(code),
        "stdout:\n{}\nstderr:\n{}",
        world.stdout,
        world.stderr
    );
}

#[then(expr = "stderr is empty")]
fn stderr_is_empty(world: &mut World) {
    assert!(world.stderr.is_empty(), "{}", world.stderr);
}

#[then(regex = r"^stderr is ([A-Z_]+)$")]
fn stderr_is(world: &mut World, code: String) {
    assert_eq!(world.stderr.trim(), code, "{}", world.stdout);
}

#[then(expr = "stdout is empty")]
fn stdout_is_empty(world: &mut World) {
    assert!(world.stdout.is_empty(), "{}", world.stdout);
}

#[then(expr = "world_time is 1080")]
fn world_time_is_1080(world: &mut World) {
    assert_eq!(world_of(table::pack(world)).world_time, 1080);
}

#[then(expr = "world_time is 1085")]
async fn world_time_is_1085(world: &mut World) {
    table::wait_round(world, 1085).await;
}

/// The id of the cast member or place in `canon` whose name is `text`; any other text, such as
/// an id or an exit address, as written.
pub(crate) fn resolve_id(canon: &spookery_engine::testing::Canon, text: &str) -> String {
    let cast = canon.cast.keys().map(spookery_engine::ActorId::as_str);
    let mut ids = cast.chain(canon.places.keys().map(spookery_engine::PlaceId::as_str));
    let named = ids.find(|id| canon.name(id) == Some(text));
    named.unwrap_or(text).to_string()
}

#[given(regex = r#"^(?P<actor>\S+) is (?:in|at) (?P<place>\S+|"[^"]+")$"#)]
#[then(regex = r#"^(?P<actor>\S+) is (?:in|at) (?P<place>\S+|"[^"]+")$"#)]
fn is_in(world: &mut World, actor: String, place: String) {
    let now = world_of(table::pack(world));
    let [actor, place] =
        [&actor, &place].map(|text| resolve_id(now.canon(), text.trim_matches('"')));
    assert_eq!(place_of(&now, &actor), place);
}

#[then(regex = r"^(?P<actor>\S+) (?P<has>has|does not have) flag (?P<flag>\S+)$")]
fn has_flag(world: &mut World, actor: String, has: String, flag: String) {
    let now = world_of(table::pack(world));
    let actor = resolve_id(now.canon(), &actor);
    let flags = flags_of(&now, &actor);
    assert_eq!(flags.contains(&flag), has == "has", "{actor}: {flags:?}");
}

#[when(regex = r"^the player marks (?P<actor>\S+)$")]
async fn the_player_marks(world: &mut World, actor: String) {
    let canon = spookery_engine::load_canon(table::pack(world))
        .unwrap_or_else(|err| panic!("load: {err:?}"));
    let line = format!("mark {}", resolve_id(&canon, &actor));
    let (mark, added) = table::player_send(world, &line, "\"kind\":").await;
    world.mark = mark;
    world.player_added = added;
}

#[when(expr = "the player waits")]
async fn the_player_waits(world: &mut World) {
    let (mark, added) = table::player_send(world, "wait", "\"kind\":").await;
    world.mark = mark;
    world.player_added = added;
}

#[when(expr = "the player travels to {string}")]
async fn the_player_travels_to(world: &mut World, place: String) {
    let (mark, added) = table::player_send(world, &format!("travel {place}"), "\"kind\":").await;
    world.mark = mark;
    world.player_added = added;
}

#[when(expr = "the GM commits the round")]
async fn the_gm_commits_the_round(world: &mut World) {
    world.stdout = table::gm_tool_call(world, 2, "commit", r#"{"include_round":true}"#).await;
}

#[then(expr = "one round was played")]
fn one_round_was_played(world: &mut World) {
    assert_eq!(world_of(table::pack(world)).round_index, 1);
}

#[then(expr = "no round was played")]
fn no_round_was_played(world: &mut World) {
    assert_eq!(world_of(table::pack(world)).round_index, 0);
}

#[then(expr = "save.yaml is unchanged since the seat")]
fn save_is_unchanged_since_the_seat(world: &mut World) {
    let seated = table::live_mut(world).seated_save.clone();
    let now = fs::read(table::pack(world).join("state/save.yaml"))
        .unwrap_or_else(|err| panic!("read save.yaml: {err}"));
    assert_eq!(now, seated);
}

#[then(expr = "spook verify passes")]
async fn spook_verify_passes(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::run(world, &["verify", pack.to_str().unwrap()]).await;
    assert_eq!(
        world.code,
        Some(0),
        "stdout:\n{}\nstderr:\n{}",
        world.stdout,
        world.stderr
    );
}

#[when(expr = "the operator runs verify")]
async fn the_operator_runs_verify(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::run(world, &["verify", pack.to_str().unwrap()]).await;
}

#[given(expr = "the trapdoor is hidden and rowan does not know fact_trapdoor")]
async fn the_trapdoor_is_hidden(world: &mut World) {
    let pack = copy_pack("docks", "trapdoor");
    add_trapdoor(&pack);
    edit_save(&pack, |_| {});
    table::listen(world, &pack).await;
    table::seat_rowan(world, &pack).await;
    world.pack = Some(pack);
    world.player_added = table::player_text(world);
}

/// `help` lists the exits of this same view, so it hides what the view hides.
#[then(expr = "rowan's view does not list the trapdoor")]
fn the_view_hides_the_trapdoor(world: &mut World) {
    let view = table::player_text(world);
    assert!(!view.contains("trapdoor"), "the view listed it\n{view}");
    assert!(view.contains("stairs"), "the view hid the stairs\n{view}");
}

#[given(expr = "the stairs exit costs 10 ticks")]
async fn the_stairs_cost_ten_ticks(world: &mut World) {
    let pack = copy_pack("docks", "ten-ticks");
    replace_in_file(
        &pack.join("world/places/docks_shop.md"),
        "ticks: 2\n",
        "ticks: 10\n",
    );
    edit_save(&pack, |now| {
        give_key(now);
        now.world_time = EVENING;
    });
    table::listen(world, &pack).await;
    table::seat_rowan(world, &pack).await;
    world.pack = Some(pack);
}

#[when(expr = "rowan travels via stairs at tick {int}")]
async fn rowan_travels_via_stairs(world: &mut World, tick: u64) {
    assert_eq!(world_of(table::pack(world)).world_time, tick);
    let (mark, added) = table::player_send(world, "travel stairs", "\"kind\":").await;
    world.mark = mark;
    world.player_added = added;
}
