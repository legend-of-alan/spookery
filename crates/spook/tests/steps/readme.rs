use std::path::{Path, PathBuf};
use std::process::Command;

use cucumber::{given, then, when};

use crate::World;

#[given(expr = "a clean {word} runner and a fresh clone")]
fn a_clean_runner(world: &mut World, os: String) {
    assert!(
        matches!(os.as_str(), "ubuntu" | "macos" | "windows"),
        "unknown runner {os}"
    );
    world.runner = os;
}

#[when(expr = "the README quick-start commands run with the stub GM")]
fn the_readme_commands_run(world: &mut World) {
    // The other OS rows are proven by the quick-start job on that runner.
    if !host_is(&world.runner) {
        return;
    }
    let dest = readme_play_folder();
    let made_folder = !dest.exists();
    if made_folder {
        std::fs::create_dir_all(&dest)
            .unwrap_or_else(|err| panic!("create {}: {err}", dest.display()));
    }
    let marker = dest.join("keep-me");
    std::fs::write(&marker, b"keep")
        .unwrap_or_else(|err| panic!("write {}: {err}", marker.display()));
    let bin = PathBuf::from(env!("CARGO_BIN_EXE_spook"));
    let mut command = find_python();
    command
        .arg(find_repo_root().join("scripts/readme_quickstart.py"))
        .current_dir(find_repo_root())
        .env(
            "SPOOK_QUICKSTART_BIN",
            bin.parent().expect("spook binary dir"),
        );
    let output = command
        .output()
        .unwrap_or_else(|err| panic!("run quick start: {err}"));
    world.code = output.status.code();
    world.stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    world.stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        marker.is_file(),
        "the quick start removed {}",
        dest.display()
    );
    let _ = std::fs::remove_file(&marker);
    if made_folder {
        let _ = std::fs::remove_dir_all(&dest);
    }
}

fn readme_play_folder() -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(std::env::var("TEMP").unwrap_or_else(|_| ".".to_string())).join("yard")
    } else {
        PathBuf::from("/tmp/yard")
    }
}

#[then(expr = "the session reaches the ending and spook verify passes")]
fn the_session_reaches_the_ending(world: &mut World) {
    if !host_is(&world.runner) {
        return;
    }
    assert_eq!(
        world.code,
        Some(0),
        "quick start failed\nstdout:\n{}\nstderr:\n{}",
        world.stdout,
        world.stderr
    );
    let ending = read_ending_line();
    assert!(
        world.stdout.contains(&ending),
        "the session did not print the ending {ending}\n{}",
        world.stdout
    );
    assert!(
        world.stdout.contains("verified"),
        "spook verify did not pass\n{}",
        world.stdout
    );
}

fn host_is(os: &str) -> bool {
    match os {
        "windows" => cfg!(windows),
        "macos" => cfg!(target_os = "macos"),
        "ubuntu" => cfg!(target_os = "linux"),
        _ => false,
    }
}

fn read_ending_line() -> String {
    let readme = read_repo("README.md");
    let marker = "### Ending";
    let start = readme
        .find(marker)
        .unwrap_or_else(|| panic!("README.md has no Ending section"));
    let rest = &readme[start + marker.len()..];
    let fence = rest
        .split("```")
        .nth(1)
        .unwrap_or_else(|| panic!("the Ending section has no fence"));
    fence
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with("text"))
        .unwrap_or_else(|| panic!("the Ending fence is empty"))
        .to_string()
}

fn find_python() -> Command {
    for name in ["python", "python3"] {
        if Command::new(name)
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success())
        {
            return Command::new(name);
        }
    }
    let mut command = Command::new("py");
    command.arg("-3");
    command
}

fn read_repo(rel: &str) -> String {
    let path = find_repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()))
}

fn find_repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
