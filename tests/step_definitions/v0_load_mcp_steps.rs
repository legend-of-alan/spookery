use std::time::Duration;

use cucumber::{then, when};
use rmcp::service::ServiceExt;
use rmcp::transport::TokioChildProcess;

use crate::World;

#[when(expr = "spook run starts with --mcp pack-table")]
async fn list_pack_table(w: &mut World) {
    let pack = w.pack.as_ref().unwrap().to_str().unwrap().to_string();
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_spook"));
    command.args(["run", &pack, "--mcp", "pack-table"]);
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
    let names = listed
        .tools
        .iter()
        .map(|tool| tool.name.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    w.scratch.insert("mcp-tools".to_string(), names);
    let _ = tokio::time::timeout(Duration::from_secs(5), running.cancel()).await;
}

#[then(expr = "tools\\/list order is {}")]
fn tools_order(w: &mut World, list: String) {
    let got = w.scratch.get("mcp-tools").unwrap();
    let expected = list.split(", ").collect::<Vec<_>>().join("\n");
    assert_eq!(got, &expected);
}

#[then(expr = "tools\\/list does not include say")]
fn tools_omit_say(w: &mut World) {
    let names = w.scratch.get("mcp-tools").unwrap();
    assert!(names.lines().all(|name| name != "say"));
}
