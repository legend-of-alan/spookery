use std::fs;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use cucumber::{given, then, when};

use crate::support::{self, text_of, wait_exit};
use crate::World;

fn files(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries =
            fs::read_dir(&dir).unwrap_or_else(|err| panic!("read {}: {err}", dir.display()));
        for entry in entries {
            let entry = entry.unwrap_or_else(|err| panic!("dirent {}: {err}", dir.display()));
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let bytes =
                    fs::read(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
                out.push((path, bytes));
            }
        }
    }
    out.sort();
    out
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0xf) as usize] as char);
    }
    out
}

fn file_key(path: &Path) -> String {
    format!("bind-file:{}", path.display())
}

fn pack_arg(w: &World) -> String {
    w.pack
        .as_ref()
        .unwrap()
        .to_str()
        .unwrap_or_else(|| panic!("pack path is not utf-8"))
        .to_string()
}

fn run(args: &[&str]) -> std::process::Output {
    let mut proc = support::spawn(args);
    let status = wait_exit(&mut proc, Duration::from_secs(5));
    let stdout = text_of(&proc.stdout).into_bytes();
    let stderr = text_of(&proc.stderr).into_bytes();
    std::process::Output {
        status,
        stdout,
        stderr,
    }
}

fn output_text(output: &std::process::Output) -> (String, String) {
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[given(expr = "the 9737 port lock is held")]
fn hold_port(w: &mut World) {
    w.port = Some(crate::PortGuard(support::lock_port()));
}

#[given(expr = "the pack file bytes are recorded")]
fn record_pack_files(w: &mut World) {
    let pack = w.pack.as_ref().unwrap().clone();
    let listed = files(&pack);
    w.scratch
        .insert("bind-file-count".to_string(), listed.len().to_string());
    for (path, bytes) in listed {
        w.scratch.insert(file_key(&path), hex(&bytes));
    }
}

#[given(expr = "a directory that holds only that pack.yaml")]
fn yaml_only(w: &mut World) {
    let pack = w.pack.as_ref().unwrap();
    let dir = std::env::temp_dir().join(format!("spook-yaml-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap_or_else(|err| panic!("mkdir {}: {err}", dir.display()));
    fs::copy(pack.join("pack.yaml"), dir.join("pack.yaml"))
        .unwrap_or_else(|err| panic!("copy pack.yaml: {err}"));
    w.scratch.insert(
        "only_yaml".to_string(),
        dir.to_str()
            .unwrap_or_else(|| panic!("yaml dir is not utf-8"))
            .to_string(),
    );
}

#[when(expr = "spook run starts with --as player:pc_rowan")]
fn run_as(w: &mut World) {
    let pack = pack_arg(w);
    w.outputs
        .push(run(&["run", &pack, "--as", "player:pc_rowan"]));
}

#[when(expr = "spook run --bind is tried for 0.0.0.0 and localhost")]
fn run_bind_rejected(w: &mut World) {
    let pack = pack_arg(w);
    for addr in ["0.0.0.0", "localhost"] {
        w.outputs.push(run(&["run", &pack, "--bind", addr]));
    }
}

#[when(expr = "spook run --bind 127.0.0.1:9737 listens")]
fn run_bind_loopback_port(w: &mut World) {
    let pack = pack_arg(w);
    let mut proc = support::spawn(&["run", &pack, "--bind", "127.0.0.1:9737"]);
    support::wait_listen(&mut proc);
    support::write_stdin(&mut proc, "stop");
    let status = wait_exit(&mut proc, Duration::from_secs(5));
    w.scratch
        .insert("bind-accepted".to_string(), status.code().unwrap_or(1).to_string());
    w.scratch
        .insert("bind-accepted-stderr".to_string(), text_of(&proc.stderr));
}

#[then(expr = "that loopback bind was accepted")]
fn bind_accepted(w: &mut World) {
    let code = w.scratch.get("bind-accepted").map(String::as_str);
    let stderr = w
        .scratch
        .get("bind-accepted-stderr")
        .map(String::as_str)
        .unwrap_or("");
    assert_eq!(code, Some("0"), "{stderr}");
    assert!(!stderr.contains("NOT_IN_SPEC"), "{stderr}");
}

#[when(expr = "spook run starts with --mcp pack-table and --bind 127.0.0.1")]
fn run_mcp_and_bind(w: &mut World) {
    let pack = pack_arg(w);
    w.outputs.push(run(&[
        "run",
        &pack,
        "--mcp",
        "pack-table",
        "--bind",
        "127.0.0.1",
    ]));
}

#[when(expr = "spook run binds while 127.0.0.1:9737 is held")]
fn run_bind_in_use(w: &mut World) {
    let _held =
        TcpListener::bind("127.0.0.1:9737").unwrap_or_else(|err| panic!("hold 9737: {err}"));
    let again = TcpListener::bind("127.0.0.1:9737").unwrap_err();
    w.text = again.to_string();
    let pack = pack_arg(w);
    w.outputs.push(run(&["run", &pack]));
}

#[when(
    expr = "spook play, spook play on the yaml-only directory, spook gm, spook gm --stdio on the yaml-only directory, and spook play --as player:pc_rowan run with nothing listening"
)]
fn clients_without_listener(w: &mut World) {
    let expected = TcpStream::connect("127.0.0.1:9737").unwrap_err();
    w.text = expected.to_string();
    let pack = pack_arg(w);
    let only = w.scratch.get("only_yaml").unwrap().clone();
    let commands = [
        vec!["play".to_string(), pack.clone()],
        vec!["play".to_string(), only.clone()],
        vec!["gm".to_string(), pack.clone()],
        vec!["gm".to_string(), only, "--stdio".to_string()],
        vec![
            "play".to_string(),
            pack,
            "--as".to_string(),
            "player:pc_rowan".to_string(),
        ],
    ];
    for args in &commands {
        let owned: Vec<&str> = args.iter().map(String::as_str).collect();
        w.outputs.push(run(&owned));
    }
}

#[then(expr = "the command exits 1")]
fn command_exits_1(w: &mut World) {
    let output = w.outputs.last().unwrap();
    let (stdout, stderr) = output_text(output);
    assert_eq!(
        output.status.code(),
        Some(1),
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

#[then(expr = "each command exits 1")]
fn each_command_exits_1(w: &mut World) {
    for output in &w.outputs {
        let (stdout, stderr) = output_text(output);
        assert_eq!(
            output.status.code(),
            Some(1),
            "stdout:\n{stdout}\nstderr:\n{stderr}"
        );
    }
}

#[then(expr = "the command stderr is NOT_IN_SPEC")]
fn command_stderr_not_in_spec(w: &mut World) {
    let output = w.outputs.last().unwrap();
    let (_, stderr) = output_text(output);
    assert_eq!(stderr.trim(), "NOT_IN_SPEC");
}

#[then(expr = "each command stderr is NOT_IN_SPEC")]
fn each_stderr_not_in_spec(w: &mut World) {
    for output in &w.outputs {
        let (_, stderr) = output_text(output);
        assert_eq!(stderr.trim(), "NOT_IN_SPEC");
    }
}

#[then(expr = "the command stdout is empty")]
fn command_stdout_empty(w: &mut World) {
    let output = w.outputs.last().unwrap();
    assert!(output.stdout.is_empty(), "{}", output_text(output).0);
}

#[then(expr = "each command stdout is empty")]
fn each_stdout_empty(w: &mut World) {
    for output in &w.outputs {
        let (stdout, _) = output_text(output);
        assert!(output.stdout.is_empty(), "{stdout}");
    }
}

#[then(expr = "the command stderr is the operating-system error")]
fn command_stderr_is_os_error(w: &mut World) {
    let output = w.outputs.last().unwrap();
    let (_, stderr) = output_text(output);
    assert_eq!(stderr.trim(), w.text);
}

#[then(expr = "each command stderr is the operating-system error")]
fn each_stderr_is_os_error(w: &mut World) {
    for output in &w.outputs {
        let (_, stderr) = output_text(output);
        assert_eq!(stderr.trim(), w.text);
    }
}

#[then(expr = "the command stderr does not contain NOT_IN_SPEC")]
fn command_stderr_omits_not_in_spec(w: &mut World) {
    let output = w.outputs.last().unwrap();
    let (_, stderr) = output_text(output);
    assert!(!stderr.contains("NOT_IN_SPEC"), "{stderr}");
}

#[then(expr = "each command stderr does not contain NOT_IN_SPEC")]
fn each_stderr_omits_not_in_spec(w: &mut World) {
    for output in &w.outputs {
        let (_, stderr) = output_text(output);
        assert!(!stderr.contains("NOT_IN_SPEC"), "{stderr}");
    }
}

#[then(expr = "the recorded pack file bytes are unchanged")]
fn pack_bytes_unchanged(w: &mut World) {
    let pack = w.pack.as_ref().unwrap();
    let listed = files(pack);
    let count: usize = w.scratch.get("bind-file-count").unwrap().parse().unwrap();
    assert_eq!(listed.len(), count, "pack file count changed");
    for (path, bytes) in listed {
        let saved = w
            .scratch
            .get(&file_key(&path))
            .unwrap_or_else(|| panic!("missing {}", path.display()));
        assert_eq!(saved, &hex(&bytes), "{}", path.display());
    }
    if let Some(only) = w.scratch.get("only_yaml") {
        let _ = fs::remove_dir_all(only);
    }
}

#[then(expr = "port 9737 is free")]
fn port_is_free(_w: &mut World) {
    let _listener = TcpListener::bind("127.0.0.1:9737")
        .unwrap_or_else(|err| panic!("listener still open: {err}"));
}
