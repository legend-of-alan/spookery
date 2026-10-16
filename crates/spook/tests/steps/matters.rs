use std::fs;
use std::path::{Path, PathBuf};

use cucumber::{given, then, when};
use spookery_kernel::api::model::Presence;
use spookery_kernel::api::World as Shard;

use crate::World;

const SECRET_ID: &str = "fact_sealed_name";
const SECRET_TEXT: &str = "the sealed name";
const PUBLIC_FLAG: &str = "gate_marked";

#[given(expr = "sample-d20 with player {string} seated facing {string}")]
fn sample_d20_with_lark_facing_briar(world: &mut World, player: String, facing: String) {
    assert_eq!(player, "lark");
    assert_eq!(facing, "briar");
    world.pack = Some(copy_sample("matters"));
}

#[given(expr = "lark has a hidden meter")]
fn lark_has_a_hidden_meter(world: &mut World) {
    let pack = pack(world);
    let mut meters = fs::read_to_string(pack.join("rules/meters.yaml")).unwrap();
    meters.push_str(
        "  - id: omen\n    min: 0\n    base_cap: 5\n    max: 5\n    start: 1\n    hidden: true\n",
    );
    fs::write(pack.join("rules/meters.yaml"), meters).unwrap();
    let path = pack.join("state/save.yaml");
    let text = fs::read_to_string(&path).unwrap();
    let from = "      might:\n        base: 40\n        offset: 0\n";
    assert_eq!(text.matches(from).count(), 1, "lark's might was not unique");
    let next = text.replacen(
        from,
        "      might:\n        base: 40\n        offset: 0\n      omen:\n        base: 1\n        offset: 0\n",
        1,
    );
    fs::write(&path, next).unwrap();
    stamp(pack);
}

#[given(expr = "a round that changes a hidden fact")]
fn a_round_that_changes_a_hidden_fact(world: &mut World) {
    let pack = pack(world).to_path_buf();
    let mut cast = fs::read_to_string(pack.join("cast/briar.md")).unwrap();
    let from = "facts: []\n";
    assert!(cast.contains(from), "{cast}");
    let fact = format!("facts:\n  - id: {SECRET_ID}\n    text: {SECRET_TEXT}\n    hidden: true\n");
    cast = cast.replacen(from, &fact, 1);
    fs::write(pack.join("cast/briar.md"), cast).unwrap();
    let mut verbs = fs::read_to_string(pack.join("rules/verbs.yaml")).unwrap();
    verbs.push_str(&format!(
        "  - id: spill\n    when:\n      verb: spill\n    do:\n      - reveal:\n          who: npc_briar\n          fact: {SECRET_ID}\n      - flag:\n          who: world\n          key: {SECRET_ID}\n          value: true\n      - flag:\n          who: world\n          key: {PUBLIC_FLAG}\n          value: true\n"
    ));
    fs::write(pack.join("rules/verbs.yaml"), verbs).unwrap();
    stamp(&pack);
    let mut shard = seat(&pack);
    play(&mut shard, "spill");
    world.summary = shard.change_summary().to_vec();
    world.stdout = change_line(&mut shard);
}

#[when(expr = "lark strikes briar and the seeded roll succeeds")]
fn lark_strikes_briar(world: &mut World) {
    let pack = pack(world).to_path_buf();
    let mut shard = seat(&pack);
    play(&mut shard, "strike npc_briar");
    world.stdout = change_line(&mut shard);
}

#[when(expr = "lark uses a tonic")]
fn lark_uses_a_tonic(world: &mut World) {
    let pack = pack(world).to_path_buf();
    let mut shard = seat(&pack);
    play(&mut shard, "use tonic");
    world.stdout = change_line(&mut shard);
}

#[when(expr = "lark looks at their view")]
fn lark_looks_at_their_view(world: &mut World) {
    let pack = pack(world).to_path_buf();
    let shard = seat(&pack);
    world.stdout = shown_text(&shard);
}

#[then(expr = "lark sees one line naming the change to briar's hp")]
fn lark_sees_briars_hp_change(world: &mut World) {
    let line = one_line(&world.stdout);
    let lower = line.to_lowercase();
    assert!(lower.contains("briar") && lower.contains("hp"), "{line}");
    assert!(!line.contains("actor_meter"), "{line}");
    assert!(line.contains('4'), "{line}");
}

#[then(expr = "lark sees one line saying the tonic was used up")]
fn lark_sees_the_tonic_used_up(world: &mut World) {
    let line = one_line(&world.stdout);
    let lower = line.to_lowercase();
    assert!(
        lower.contains("tonic") && lower.contains("used up"),
        "{line}"
    );
}

#[then(expr = "lark sees lark's hp and might")]
fn lark_sees_hp_and_might(world: &mut World) {
    assert!(world.stdout.contains("hp: 8"), "{}", world.stdout);
    assert!(world.stdout.contains("might: 40"), "{}", world.stdout);
    assert!(!world.stdout.contains("might: 2"), "{}", world.stdout);
}

#[then(expr = "lark does not see a hidden meter")]
fn lark_does_not_see_a_hidden_meter(world: &mut World) {
    assert!(
        !world.stdout.to_lowercase().contains("omen"),
        "{}",
        world.stdout
    );
}

#[then(expr = "the player's change line does not mention it")]
fn the_change_line_hides_the_fact(world: &mut World) {
    assert!(
        world.summary.iter().any(|item| item.contains(SECRET_ID)),
        "{:?}",
        world.summary
    );
    assert!(
        world.summary.iter().any(|item| item.contains(PUBLIC_FLAG)),
        "{:?}",
        world.summary
    );
    let line = one_line(&world.stdout);
    assert!(line.contains(PUBLIC_FLAG), "{line}");
    assert!(!line.contains(SECRET_ID), "{line}");
    assert!(!line.contains(SECRET_TEXT), "{line}");
}

fn one_line(text: &str) -> &str {
    assert!(!text.is_empty() && !text.contains('\n'), "{text}");
    text
}

fn pack(world: &World) -> &Path {
    world.pack.as_deref().unwrap_or_else(|| panic!("no pack"))
}

fn copy_sample(label: &str) -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let pack = std::env::temp_dir().join(format!(
        "spook-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&pack);
    crate::support::copy_dir(&manifest.join("../../packs/sample-d20"), &pack);
    pack
}

fn stamp(pack: &Path) {
    let path = pack.join("state/save.yaml");
    let text = fs::read_to_string(&path).unwrap();
    let mut now = spookery_kernel::api::store::codec::load_save(&text).unwrap_or_else(|diags| {
        panic!(
            "load save:\n{}",
            diags
                .iter()
                .map(|diag| diag.report_line())
                .collect::<Vec<_>>()
                .join("\n")
        )
    });
    now.pack_rev = spookery_kernel::api::canon::pack_rev(pack).unwrap();
    let emitted = spookery_kernel::api::store::codec::emit_save(&now).unwrap();
    fs::write(path, emitted).unwrap();
}

fn seat(pack: &Path) -> Shard {
    let mut world = match Shard::open(pack) {
        Ok(world) => world,
        Err(err) => {
            let report = spookery_kernel::api::check_pack(pack)
                .iter()
                .map(|diag| diag.report_line())
                .collect::<Vec<_>>()
                .join("\n");
            panic!("open: {}\n{report}", err.token());
        }
    };
    world.set_presence(Presence {
        humans: 1,
        actors_present: vec!["pc_lark".to_string()],
    });
    world
}

fn play(world: &mut Shard, line: &str) {
    spookery::host::handle_player_move(world, "pc_lark", line, false)
        .unwrap_or_else(|err| panic!("{line}: {}", err.token()));
}

fn change_line(world: &mut Shard) -> String {
    world
        .take_seen()
        .into_iter()
        .find(|seen| seen.viewer == "pc_lark")
        .map(|seen| seen.line)
        .unwrap_or_default()
}

fn shown_text(world: &Shard) -> String {
    let view = spookery_kernel::api::perceive(world, "player:pc_lark")
        .unwrap_or_else(|err| panic!("perceive: {}", err.token()));
    view.meters
        .iter()
        .map(|meter| format!("{}: {}", meter.id, meter.value))
        .collect::<Vec<_>>()
        .join("\n")
}
