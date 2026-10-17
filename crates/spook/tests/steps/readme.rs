use std::path::PathBuf;
use std::process::Command;

use cucumber::{given, then, when};

use crate::contributing::{find_repo_root, read_repo};
use crate::World;

#[given(expr = "a clean {word} runner and a fresh clone")]
fn a_clean_runner(world: &mut World, os: String) {
    let workflow = read_repo(".github/workflows/ci.yml");
    let runner = match os.as_str() {
        "ubuntu" => "ubuntu-latest",
        "macos" => "macos-latest",
        "windows" => "windows-latest",
        other => panic!("unknown runner {other}"),
    };
    assert!(
        workflow.contains(runner),
        "the quick-start job does not run on {runner}"
    );
    assert!(
        workflow.contains("scripts/readme_quickstart.py"),
        "the quick-start job does not run the README commands"
    );
    assert!(
        !workflow
            .lines()
            .any(|line| line.trim() == "- \"README.md\""),
        "README.md is still in paths-ignore"
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

#[then(expr = "the docs have one how-this-differs paragraph")]
fn the_docs_have_one_how_this_differs_paragraph(_world: &mut World) {
    let text = read_repo("docs/README.md");
    let start = text
        .find("## How this differs")
        .unwrap_or_else(|| panic!("docs/README.md has no How this differs section"));
    let rest = &text[start + "## How this differs".len()..];
    let body = rest.split("\n## ").next().unwrap_or("").trim();
    let paragraphs: Vec<&str> = body
        .split("\n\n")
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect();
    assert_eq!(
        paragraphs.len(),
        1,
        "How this differs should be one paragraph:\n{body}"
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
