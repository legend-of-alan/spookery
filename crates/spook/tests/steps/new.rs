use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use cucumber::{given, then, when};
use spookery_kernel::model::ActorLive;

use crate::support;
use crate::World;

use super::table;

#[when(expr = "the operator runs spook new in a fresh directory")]
fn the_operator_runs_spook_new_in_a_fresh_directory(world: &mut World) {
    let pack = fresh_path("pack");
    run_new(world, &pack);
    assert_eq!(world.code, Some(0), "{}", world.stderr);
    world.pack = Some(pack);
}

#[then(expr = "stdout names the next commands")]
fn stdout_names_the_next_commands(world: &mut World) {
    let pack = table::pack(world);
    let text = pack.to_str().unwrap();
    for line in [
        format!("Wrote {text}"),
        format!("spook check {text}"),
        format!("spook run {text}"),
        format!("spook play {text}"),
    ] {
        assert!(
            world.stdout.contains(&line),
            "missing {line}\n{}",
            world.stdout
        );
    }
}

#[then(expr = "the written files match the starter template")]
fn the_written_files_match_the_starter_template(world: &mut World) {
    let pack = table::pack(world);
    assert_eq!(relative_files(pack), relative_files(&template_dir()));
}

#[given(expr = "a non-empty directory")]
fn a_non_empty_directory(world: &mut World) {
    let dir = fresh_path("full");
    fs::create_dir(&dir).unwrap_or_else(|err| panic!("mkdir: {err}"));
    fs::write(dir.join("keep.txt"), b"keep\n").unwrap();
    world.pack = Some(dir);
    world.before = table::files(table::pack(world));
}

#[when(expr = "the operator runs spook new on that directory")]
fn the_operator_runs_spook_new_on_that_directory(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    run_new(world, &pack);
}

#[when(expr = "the operator runs spook new under a missing parent")]
fn the_operator_runs_spook_new_under_a_missing_parent(world: &mut World) {
    let parent = fresh_path("missing");
    let pack = parent.join("pack");
    run_new(world, &pack);
    world.pack = Some(parent);
}

#[then(expr = "stderr names the missing parent")]
fn stderr_names_the_missing_parent(world: &mut World) {
    let parent = table::pack(world);
    let expected = format!("{} does not exist", parent.display());
    assert_eq!(world.stderr.trim(), expected, "{}", world.stderr);
}

#[then(expr = "the missing parent was not created")]
fn the_missing_parent_was_not_created(world: &mut World) {
    let parent = table::pack(world);
    assert!(!parent.exists(), "{}", parent.display());
}

#[when(expr = "the operator runs spook new beside a marker file")]
fn the_operator_runs_spook_new_beside_a_marker_file(world: &mut World) {
    let parent = fresh_path("sandbox");
    fs::create_dir(&parent).unwrap_or_else(|err| panic!("mkdir: {err}"));
    fs::write(parent.join("keep.txt"), b"keep\n").unwrap();
    world.before = table::files(&template_dir());
    world.now_bytes = fs::read(parent.join("keep.txt")).unwrap();
    let pack = parent.join("pack");
    run_new(world, &pack);
    world.scratch = pack.to_str().unwrap().to_string();
    world.pack = Some(parent);
}

#[then(expr = "the marker file is unchanged")]
fn the_marker_file_is_unchanged(world: &mut World) {
    let parent = table::pack(world);
    let bytes = fs::read(parent.join("keep.txt")).unwrap();
    assert_eq!(bytes, world.now_bytes);
}

#[then(expr = "the parent gained only the new pack")]
fn the_parent_gained_only_the_new_pack(world: &mut World) {
    let parent = table::pack(world);
    let mut names = fs::read_dir(parent)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    names.sort();
    assert_eq!(names, vec!["keep.txt".to_string(), "pack".to_string()]);
    let pack = parent.join("pack");
    assert_eq!(relative_files(&pack), relative_files(&template_dir()));
}

#[then(expr = "the starter template is unchanged")]
fn the_starter_template_is_unchanged(world: &mut World) {
    assert_eq!(table::files(&template_dir()), world.before);
}

#[when(expr = "the operator runs spook check --json on the starter template")]
fn the_operator_runs_spook_check_json_on_the_starter_template(world: &mut World) {
    let pack = template_dir();
    table::run(world, &["check", "--json", pack.to_str().unwrap()]);
}

#[when(expr = "play joins that pack on a free loopback address")]
fn play_joins_that_pack_on_a_free_loopback_address(world: &mut World) {
    let port = table::free_port();
    assert_ne!(port, 9737);
    let addr = format!("127.0.0.1:{port}");
    let pack = table::pack(world);
    table::listen(pack, &addr, port);
    let pack_arg = support::pack_arg(pack);
    world.play = Some(support::spawn(&["play", &pack_arg, "--addr", &addr]));
    wait_play(world, "You are in Yard.");
}

#[then(expr = "the travel committed a tick")]
fn the_travel_committed_a_tick(world: &mut World) {
    let pack = table::pack(world);
    table::wait_world_time(pack, 5);
    let now = table::now_of(pack);
    assert_eq!(now.actors.get("pc_ada").unwrap().place(), "lane");
    assert!(
        pack.join("state/journal/tick-0001.jsonl").is_file(),
        "no tick file"
    );
}

#[when(expr = "the operator runs spook new beside that pack")]
fn the_operator_runs_spook_new_beside_that_pack(world: &mut World) {
    let pack = table::pack(world);
    let name = pack.file_name().unwrap().to_string_lossy();
    let idle = pack.parent().unwrap().join(format!("{name}-idle"));
    let _ = fs::remove_dir_all(&idle);
    run_new(world, &idle);
    assert_eq!(world.code, Some(0), "{}", world.stderr);
    world.scratch = idle.to_str().unwrap().to_string();
}

#[when(expr = "the operator runs tick on that untouched pack")]
fn the_operator_runs_tick_on_that_untouched_pack(world: &mut World) {
    let idle = PathBuf::from(&world.scratch);
    table::run(world, &["tick", idle.to_str().unwrap()]);
}

#[then(expr = "the keeper habit has set tended")]
fn the_keeper_habit_has_set_tended(world: &mut World) {
    let idle = PathBuf::from(&world.scratch);
    assert_eq!(world.code, Some(0), "{}", world.stderr);
    let now = table::now_of(&idle);
    assert_eq!(now.world_time, 60, "idle tick did not advance an hour");
    let flags = match now.actors.get("keeper") {
        Some(ActorLive::Npc { flags, .. }) => flags,
        other => panic!("keeper missing: {other:?}"),
    };
    assert!(flags.iter().any(|flag| flag == "tended"), "{flags:?}");
    let _ = fs::remove_dir_all(idle);
}

#[when(expr = "the first-pack guide steps are applied in order")]
fn the_first_pack_guide_steps_are_applied_in_order(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    let edits = guide_edits();
    assert!(edits.len() >= 4, "the guide has {} steps", edits.len());
    assert!(edits.iter().any(|edit| edit.mode == "create"));
    assert!(edits.iter().any(|edit| edit.mode == "append"));
    assert!(edits.iter().any(|edit| edit.mode == "insert-before"));
    for edit in &edits {
        apply_edit(&pack, edit);
    }
    let arg = support::pack_arg(&pack);
    table::run(world, &["check", "--json", &arg]);
    assert_eq!(world.code, Some(1), "{}", world.stdout);
    assert!(
        world.stdout.contains("PACK_REV_MISMATCH"),
        "{}",
        world.stdout
    );
    table::run(world, &["migrate", &arg]);
    assert_eq!(world.code, Some(0), "{}", world.stderr);
    table::run(world, &["check", "--json", &arg]);
}

struct Edit {
    path: String,
    mode: String,
    anchor: Option<String>,
    body: String,
}

fn guide_edits() -> Vec<Edit> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/first-pack.md");
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read guide: {err}"));
    let text = text.replace('\r', "");
    let mut edits = Vec::new();
    let mut rest = text.as_str();
    let opener = "```yaml path=";
    while let Some(start) = rest.find(opener) {
        let after = &rest[start + "```".len()..];
        let (info, tail) = after
            .split_once('\n')
            .unwrap_or_else(|| panic!("unclosed guide fence"));
        let end = tail
            .find("\n```")
            .unwrap_or_else(|| panic!("guide fence is not closed:\n{info}"));
        let body = tail[..end].to_string();
        edits.push(parse_edit(info, body));
        rest = &tail[end + "\n```".len()..];
    }
    assert!(!edits.is_empty(), "the guide has no applied steps");
    edits
}

fn parse_edit(info: &str, body: String) -> Edit {
    let info = info
        .strip_prefix("yaml ")
        .unwrap_or_else(|| panic!("{info}"));
    let (path, rest) = info
        .strip_prefix("path=")
        .and_then(|text| text.split_once(' '))
        .unwrap_or_else(|| panic!("guide fence needs a path: {info}"));
    let (mode, anchor) = if let Some(rest) = rest.strip_prefix("mode=") {
        if let Some((mode, anchor)) = rest.split_once(" anchor=") {
            (mode.to_string(), Some(anchor.to_string()))
        } else {
            (rest.to_string(), None)
        }
    } else {
        panic!("guide fence needs a mode: {info}");
    };
    assert!(
        !path.is_empty() && !path.starts_with('/') && !path.contains("..") && !path.contains(':'),
        "guide path escapes the pack: {path}"
    );
    Edit {
        path: path.to_string(),
        mode,
        anchor,
        body,
    }
}

fn apply_edit(pack: &Path, edit: &Edit) {
    let path = pack.join(&edit.path);
    let mut body = edit.body.replace('\r', "");
    if !body.ends_with('\n') {
        body.push('\n');
    }
    match edit.mode.as_str() {
        "create" => {
            assert!(!path.exists(), "{} already exists", edit.path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(path, body).unwrap();
        }
        "append" => {
            let mut text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("{err}"));
            if !text.ends_with('\n') {
                text.push('\n');
            }
            text.push_str(&body);
            fs::write(path, text).unwrap();
        }
        "insert-before" => {
            let anchor = edit
                .anchor
                .as_deref()
                .unwrap_or_else(|| panic!("no anchor"));
            let text = fs::read_to_string(&path).unwrap();
            fs::write(path, insert_before(&text, anchor, &body)).unwrap();
        }
        other => panic!("unknown guide mode {other}"),
    }
}

fn insert_before(text: &str, anchor: &str, body: &str) -> String {
    let mut out = String::new();
    let mut found = false;
    for line in text.split_inclusive('\n') {
        let bare = line.trim_end_matches(['\n', '\r']);
        if !found && bare == anchor {
            found = true;
            out.push_str(body);
            if !body.ends_with('\n') {
                out.push('\n');
            }
        }
        out.push_str(line);
    }
    assert!(found, "anchor not in the file: {anchor:?}");
    out
}

fn run_new(world: &mut World, pack: &Path) {
    table::run(world, &["new", pack.to_str().unwrap()]);
}

fn fresh_path(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "spook-new-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&path);
    assert!(!path.exists(), "{}", path.display());
    path
}

fn template_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("templates/starter")
}

fn relative_files(root: &Path) -> Vec<(String, Vec<u8>)> {
    table::files(root)
        .into_iter()
        .map(|(path, bytes)| {
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .components()
                .map(|component| component.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            (rel, bytes)
        })
        .collect()
}

fn wait_play(world: &mut World, needle: &str) {
    let start = Instant::now();
    loop {
        let play = world.play.as_ref().unwrap_or_else(|| panic!("no play"));
        let stdout = support::text_of(&play.stdout);
        if stdout.contains(needle) {
            world.stdout = stdout;
            return;
        }
        if let Some(status) = world
            .play
            .as_mut()
            .unwrap()
            .child
            .try_wait()
            .unwrap_or_else(|err| panic!("try_wait: {err}"))
        {
            let stderr = support::text_of(&world.play.as_ref().unwrap().stderr);
            panic!("play exited {status:?}\nstdout:\n{stdout}\nstderr:\n{stderr}");
        }
        if start.elapsed() > Duration::from_secs(8) {
            let stderr = support::text_of(&world.play.as_ref().unwrap().stderr);
            panic!("missing {needle}\nstdout:\n{stdout}\nstderr:\n{stderr}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
