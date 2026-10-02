use std::fs;
use std::time::Duration;

use cucumber::{then, when};
use rmcp::model::{CallToolRequestParam, ErrorCode};
use rmcp::service::ServiceExt;
use rmcp::transport::TokioChildProcess;
use rmcp::ServiceError;
use serde_json::Value;

use crate::World;

const TABLE_TOOLS: &[&str] = &[
    "session_open",
    "session_status",
    "perceive",
    "intent",
    "resolve_check",
    "apply_effect",
    "npc_act",
    "commit_turn",
    "recall",
    "session_save",
];

#[when(expr = "pack-table stdio lists tools and calls say and session_status")]
async fn list_and_call(w: &mut World) {
    let pack = w.pack.as_ref().unwrap();
    let before =
        fs::read(pack.join("state/now.yaml")).unwrap_or_else(|err| panic!("read now.yaml: {err}"));
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_spook"));
    command.args(["run", pack.to_str().unwrap(), "--mcp", "pack-table"]);
    let running = ()
        .serve(TokioChildProcess::new(command).unwrap_or_else(|err| panic!("spawn spook: {err}")));
    let running = tokio::time::timeout(Duration::from_secs(15), running)
        .await
        .unwrap_or_else(|_| panic!("initialize timed out"))
        .unwrap_or_else(|err| panic!("initialize: {err}"));
    let listed = tokio::time::timeout(Duration::from_secs(15), running.list_tools(None))
        .await
        .unwrap_or_else(|_| panic!("tools/list timed out"))
        .unwrap_or_else(|err| panic!("tools/list: {err}"));
    let names: Vec<String> = listed
        .tools
        .iter()
        .map(|tool| tool.name.as_ref().to_string())
        .collect();
    w.scratch
        .insert("table-names".to_string(), names.join("\n"));
    let mut schemas = Vec::new();
    for tool in &listed.tools {
        let value = Value::Object(tool.input_schema.as_ref().clone());
        schemas.push(format!(
            "{}\t{}",
            tool.name,
            serde_json::to_string(&value)
                .unwrap_or_else(|err| panic!("schema {}: {err}", tool.name))
        ));
    }
    w.scratch
        .insert("table-schemas".to_string(), schemas.join("\n"));
    for name in ["say", "session_status"] {
        let called = tokio::time::timeout(
            Duration::from_secs(15),
            running.call_tool(CallToolRequestParam {
                name: (*name).into(),
                arguments: None,
            }),
        )
        .await
        .unwrap_or_else(|_| panic!("{name} tools/call timed out"));
        let record = match called {
            Err(ServiceError::McpError(err)) => format!("code:{}", err.code.0),
            Ok(result) => format!("body:{result:?}"),
            Err(err) => format!("err:{err}"),
        };
        w.scratch.insert(format!("table-call-{name}"), record);
    }
    let after =
        fs::read(pack.join("state/now.yaml")).unwrap_or_else(|err| panic!("read now.yaml: {err}"));
    w.scratch
        .insert("table-now-before".to_string(), hex(&before));
    w.scratch.insert("table-now-after".to_string(), hex(&after));
    let session = if pack.join("state/log/session.jsonl").exists() {
        "present"
    } else {
        "absent"
    };
    w.scratch
        .insert("table-session".to_string(), session.to_string());
    let _ = tokio::time::timeout(Duration::from_secs(5), running.cancel()).await;
    let _ = fs::remove_dir_all(pack);
}

#[then(expr = "pack-table tools\\/list names are the recorded table")]
fn names_are_table(w: &mut World) {
    let got = w.scratch.get("table-names").unwrap();
    assert_eq!(got, &TABLE_TOOLS.join("\n"));
}

#[then(expr = "pack-table tools\\/list omits say")]
fn names_omit_say(w: &mut World) {
    let names = w.scratch.get("table-names").unwrap();
    assert!(names.lines().all(|name| name != "say"));
}

#[then(expr = "every pack-table tool input schema is type object")]
fn schemas_are_object(w: &mut World) {
    let expected = serde_json::json!({ "type": "object" });
    let lines = w.scratch.get("table-schemas").unwrap();
    for line in lines.lines() {
        let (name, json) = line
            .split_once('\t')
            .unwrap_or_else(|| panic!("schema row {line}"));
        let value: Value = serde_json::from_str(json).unwrap_or_else(|err| panic!("{name}: {err}"));
        assert_eq!(value, expected, "{name}");
    }
}

#[then(expr = "pack-table tools\\/call {word} is METHOD_NOT_FOUND")]
fn call_is_method_not_found(w: &mut World, name: String) {
    let record = w
        .scratch
        .get(&format!("table-call-{name}"))
        .unwrap_or_else(|| panic!("no tools/call for {name}"));
    let code = record.strip_prefix("code:").unwrap_or_else(|| {
        if let Some(body) = record.strip_prefix("body:") {
            panic!("{name} tools/call returned a body: {body}");
        }
        panic!("{name} tools/call: {record}");
    });
    let code: i32 = code
        .parse()
        .unwrap_or_else(|err| panic!("{name} code {code}: {err}"));
    assert_eq!(
        ErrorCode(code),
        ErrorCode::METHOD_NOT_FOUND,
        "{name}: {record}"
    );
}

#[then(expr = "pack-table leaves state\\/now.yaml unchanged")]
fn now_unchanged(w: &mut World) {
    let before = w.scratch.get("table-now-before").unwrap();
    let after = w.scratch.get("table-now-after").unwrap();
    assert_eq!(after, before);
}

#[then(expr = "pack-table leaves state\\/log\\/session.jsonl absent")]
fn session_absent(w: &mut World) {
    assert_eq!(w.scratch.get("table-session").unwrap(), "absent");
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
}
