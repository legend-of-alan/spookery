use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;
use spookery_kernel::perceive::StockView;
use spookery_kernel::DigestViewer;

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap_or_else(|err| panic!("mkdir {}: {err}", to.display()));
    for entry in fs::read_dir(from).unwrap_or_else(|err| panic!("read {}: {err}", from.display())) {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        let file_type = entry.file_type().unwrap();
        if file_type.is_dir() {
            copy_dir(&entry.path(), &dest);
        } else {
            fs::copy(entry.path(), &dest)
                .unwrap_or_else(|err| panic!("copy {}: {err}", entry.path().display()));
        }
    }
}

fn temp_pack(label: &str) -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixture = manifest.join("packs/oneshot-docks");
    let pack = std::env::temp_dir().join(format!(
        "spook-run-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&pack);
    copy_dir(&fixture, &pack);
    pack
}

fn files(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir).unwrap_or_else(|err| panic!("read {}: {err}", dir.display()));
        for entry in entries {
            let entry = entry.unwrap_or_else(|err| panic!("dirent {}: {err}", dir.display()));
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let bytes = fs::read(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
                out.push((path, bytes));
            }
        }
    }
    out.sort();
    out
}

fn run(pack: &Path, args: &[&str], stdin: &str) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_spook"));
    command.arg("run").arg(pack).args(args);
    command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn().unwrap_or_else(|err| panic!("spawn spook: {err}"));
    child
        .stdin
        .take()
        .unwrap_or_else(|| panic!("stdin missing"))
        .write_all(stdin.as_bytes())
        .unwrap_or_else(|err| panic!("write stdin: {err}"));
    child.wait_with_output().unwrap_or_else(|err| panic!("wait spook: {err}"))
}

fn player_waits(pack: &Path, tick: u64) -> usize {
    let path = pack.join(format!("state/log/tick-{tick:04}.jsonl"));
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
    let value: Value = serde_json::from_str(text.trim_end()).unwrap_or_else(|err| panic!("json: {err}"));
    value["intents"]
        .as_array()
        .unwrap_or_else(|| panic!("intents missing"))
        .iter()
        .filter(|intent| intent["source"] == "player" && intent["move"] == "wait" && intent["actor_id"] == "pc_rowan")
        .count()
}

fn perceive_value(view: &spookery_kernel::perceive::Perceive) -> Value {
    let messages: Vec<Value> = view
        .messages
        .iter()
        .map(|message| {
            let mut object = serde_json::Map::new();
            object.insert("id".to_string(), Value::String(message.id.clone()));
            object.insert(
                "to".to_string(),
                Value::Array(message.to.iter().cloned().map(Value::String).collect()),
            );
            if let Some(place) = &message.place {
                object.insert("place".to_string(), Value::String(place.clone()));
            }
            object.insert(
                "fact_ids".to_string(),
                Value::Array(message.fact_ids.iter().cloned().map(Value::String).collect()),
            );
            object.insert("text".to_string(), Value::String(message.text.clone()));
            Value::Object(object)
        })
        .collect();
    serde_json::json!({
        "viewer": view.viewer,
        "place": view.place,
        "visible_actors": view.visible_actors,
        "known_facts": view.known_facts.iter().map(|fact| serde_json::json!({
            "id": fact.id,
            "text": fact.text,
            "body": fact.body,
        })).collect::<Vec<_>>(),
        "messages": messages,
        "unknown_mentioned": view.unknown_mentioned,
        "clocks": view.clocks.iter().map(|clock| serde_json::json!({
            "id": clock.id,
            "value": clock.value,
            "max": clock.max,
            "paused": clock.paused,
        })).collect::<Vec<_>>(),
        "stocks": view.stocks.iter().map(|stock| serde_json::json!({
            "sku": stock.sku,
            "qty": stock.qty,
        })).collect::<Vec<_>>(),
        "schedule_hint": Value::Null,
    })
}

fn take_record<'a>(rest: &'a str, digest: &str) -> (&'a str, Value) {
    assert!(
        rest.starts_with(digest),
        "resume prefix\nexpected:\n{digest}\nactual:\n{rest}"
    );
    let rest = &rest[digest.len()..];
    let (line, after) = rest.split_once('\n').unwrap_or_else(|| panic!("perceive line missing"));
    let parsed: Value = serde_json::from_str(line).unwrap_or_else(|err| panic!("perceive json: {err}"));
    (after, parsed)
}

#[test]
fn cli_run_wait_prints_resume_and_perceive() {
    let pack = temp_pack("wait");
    let output = run(
        &pack,
        &[],
        "frob\ntravel\nplayer:nope wait\nwait\nwait\n",
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert_eq!(stderr, "UNKNOWN_MOVE\nSCHEMA\nSCHEMA\n");

    let shard = spookery_kernel::open_shard(&pack).unwrap_or_else(|err| panic!("open: {err}"));
    let now = shard.now();
    assert_eq!(now.state_rev, 2);
    assert_eq!(now.tick_index, 2);
    assert_eq!(now.world_time, 1200);
    assert_eq!(now.presence.humans, 0);
    assert_eq!(now.last_tick_at, "2026-01-01T00:00:00Z");
    assert_eq!(player_waits(&pack, 1), 1);
    assert_eq!(player_waits(&pack, 2), 1);
    assert!(!pack.join("state/.txn").exists(), "commit scratch remains");

    let digest = spookery_kernel::idle_digest(&shard, DigestViewer::Player("pc_rowan".to_string()))
        .unwrap_or_else(|err| panic!("digest: {err}"));
    let view = spookery_kernel::perceive(&shard, "player:pc_rowan")
        .unwrap_or_else(|err| panic!("perceive: {err}"));
    assert!(view.stocks.iter().any(|stock| stock == &StockView {
        sku: "lamp_oil".to_string(),
        qty: 4,
    }));
    let expected = perceive_value(&view);
    let (rest, first) = take_record(&stdout, &digest);
    let (rest, second) = take_record(rest, &digest);
    assert_eq!(first, expected);
    assert_eq!(second, expected);
    assert_eq!(rest, "");
    assert!(!digest.contains("lamp_oil"));

    let _ = fs::remove_dir_all(&pack);
}

#[test]
fn cli_run_grammar_lines_are_accepted() {
    let pack = temp_pack("grammar");
    let stdin = "\
work
travel docks_street
restock lamp_oil 1
rest -1
pay_upkeep rest 1
drift_relationship mira
spread_rumor r_184 aldric
leave_note n1 shop_day 1 pc_rowan docks_shop hello there
file_proposal p1 other docks_shop a summary
advance_clock shop_day
advance_clock assassination_debt 1
talk mira
attack mira
attempt sneak
give lamp pc_rowan
take lamp mira
player:pc_rowan wait
";
    let output = run(&pack, &["--as", "player:pc_rowan"], stdin);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{stderr}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(stderr, "");
    let shard = spookery_kernel::open_shard(&pack).unwrap_or_else(|err| panic!("open: {err}"));
    assert_eq!(shard.now().state_rev, 17);
    assert_eq!(shard.now().presence.humans, 0);
    let _ = fs::remove_dir_all(&pack);
}

#[test]
fn cli_run_bad_as_is_not_in_spec() {
    let pack = temp_pack("argv");
    let before = files(&pack);
    let output = run(&pack, &["--as", "pc_rowan"], "");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        (output.status.code(), stderr.trim()),
        (Some(1), "NOT_IN_SPEC"),
        "stdout:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(files(&pack), before, "bad argv wrote the pack");
    let _ = fs::remove_dir_all(&pack);
}

#[test]
fn cli_run_eof_leaves_the_pack() {
    let pack = temp_pack("eof");
    let before = files(&pack);
    let output = run(&pack, &[], "");
    assert!(output.status.success(), "stderr:\n{}", String::from_utf8_lossy(&output.stderr));
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    assert_eq!(files(&pack), before, "eof wrote the pack");
    let _ = fs::remove_dir_all(&pack);
}

#[test]
fn cli_run_as_rejects_a_non_pc() {
    let pack = temp_pack("as");
    let before = files(&pack);
    let output = run(&pack, &["--as", "player:mira"], "");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        (output.status.code(), stderr.trim()),
        (Some(1), "SCHEMA"),
        "stdout:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(files(&pack), before, "rejected --as wrote the pack");
    let _ = fs::remove_dir_all(&pack);
}
