use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cucumber::{then, when};
use serde_json::Value;

use crate::World;

use super::table;

#[when(expr = "the operator lists the pack-table menu")]
fn lists_the_table_menu(world: &mut World) {
    world.scratch = menu(table::pack(world), "pack-table");
}

#[when(expr = "the operator lists the pack-tick menu")]
fn lists_the_tick_menu(world: &mut World) {
    world.scratch = menu(table::pack(world), "pack-tick");
}

#[when(expr = "the operator lists the pack-studio menu")]
fn lists_the_studio_menu(world: &mut World) {
    world.scratch = menu(table::pack(world), "pack-studio");
}

#[when(expr = "the operator runs mcp pack-gm")]
fn runs_mcp_pack_gm(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::run(world, &["run", pack.to_str().unwrap(), "--mcp", "pack-gm"]);
}

#[when(expr = "the GM calls npc_act for mira")]
fn the_gm_calls_npc_act(world: &mut World) {
    world.gm_added = table::gm_call(2, "npc_act", r#"{"actor_id":"mira"}"#);
}

#[then(expr = "the menu is the table tools")]
fn the_menu_is_the_table_tools(world: &mut World) {
    assert_eq!(
        world.scratch,
        "session_status\nperceive\nintent\nnpc_act\ncommit_turn"
    );
}

#[then(expr = "the menu is the tick tools")]
fn the_menu_is_the_tick_tools(world: &mut World) {
    assert_eq!(world.scratch, "session_status\nperceive\nnpc_act");
}

#[then(expr = "the menu is empty")]
fn the_menu_is_empty(world: &mut World) {
    assert!(world.scratch.is_empty(), "{}", world.scratch);
}

struct Reap(Option<std::process::Child>);

impl Drop for Reap {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn menu(pack: &std::path::Path, profile: &str) -> String {
    let mut child = Reap(Some(
        Command::new(env!("CARGO_BIN_EXE_spook"))
            .args(["run", pack.to_str().unwrap(), "--mcp", profile])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|err| panic!("spawn spook: {err}")),
    ));
    let child = child.0.as_mut().unwrap();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let text = Arc::new(Mutex::new(String::new()));
    let err_text = Arc::new(Mutex::new(String::new()));
    drain(stdout, Arc::clone(&text));
    drain(stderr, Arc::clone(&err_text));
    let mut stdin = child.stdin.take().unwrap();
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"2024-11-05","capabilities":{{}},"clientInfo":{{"name":"proof","version":"0"}}}}}}"#
    )
    .unwrap();
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","method":"notifications/initialized"}}"#
    )
    .unwrap();
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{{}}}}"#
    )
    .unwrap();
    stdin.flush().unwrap();
    let listed = wait_id(&text, &err_text, child);
    let names = tool_names(&listed);
    names.join("\n")
}

fn tool_names(line: &str) -> Vec<String> {
    let value: Value = serde_json::from_str(line).unwrap_or_else(|err| panic!("{err}\n{line}"));
    value["result"]["tools"]
        .as_array()
        .unwrap_or_else(|| panic!("no tools\n{line}"))
        .iter()
        .map(|tool| {
            tool["name"]
                .as_str()
                .unwrap_or_else(|| panic!("tool name\n{line}"))
                .to_string()
        })
        .collect()
}

fn wait_id(
    text: &Arc<Mutex<String>>,
    err_text: &Arc<Mutex<String>>,
    child: &mut std::process::Child,
) -> String {
    let start = Instant::now();
    loop {
        let got = text.lock().unwrap().clone();
        for line in got.lines() {
            if let Ok(value) = serde_json::from_str::<Value>(line) {
                if value.get("id").and_then(Value::as_i64) == Some(2) {
                    return line.to_string();
                }
            }
        }
        if let Some(status) = child.try_wait().unwrap() {
            panic!(
                "mcp exited {:?}\n{}\n{}",
                status.code(),
                got,
                err_text.lock().unwrap()
            );
        }
        if start.elapsed() > Duration::from_secs(15) {
            panic!("tools/list timed out\n{got}\n{}", err_text.lock().unwrap());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn drain(reader: impl Read + Send + 'static, slot: Arc<Mutex<String>>) {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        let mut line = String::new();
        while reader.read_line(&mut line).unwrap_or(0) > 0 {
            slot.lock().unwrap().push_str(&line);
            line.clear();
        }
    });
}
