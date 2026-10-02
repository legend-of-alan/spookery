use std::time::Duration;

use cucumber::{then, when};
use rmcp::service::ServiceExt;
use rmcp::transport::TokioChildProcess;

use crate::support;
use crate::World;

#[when(expr = "spook run lists tools for --mcp {}")]
async fn list_profile(w: &mut World, profile: String) {
    let pack = w.pack.as_ref().unwrap().to_str().unwrap().to_string();
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_spook"));
    command.args(["run", &pack, "--mcp", &profile]);
    let running = ()
        .serve(TokioChildProcess::new(command).unwrap_or_else(|err| panic!("spawn spook: {err}")));
    let running = tokio::time::timeout(Duration::from_secs(15), running)
        .await
        .unwrap_or_else(|_| panic!("{profile} initialize timed out"))
        .unwrap_or_else(|err| panic!("{profile} initialize: {err}"));
    let listed = tokio::time::timeout(Duration::from_secs(15), running.list_tools(None))
        .await
        .unwrap_or_else(|_| panic!("{profile} tools/list timed out"))
        .unwrap_or_else(|err| panic!("{profile} tools/list: {err}"));
    let names = listed
        .tools
        .iter()
        .map(|tool| tool.name.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    w.scratch.insert("mcp-tools".to_string(), names);
    let _ = tokio::time::timeout(Duration::from_secs(5), running.cancel()).await;
}

#[when(expr = "spook run starts with --mcp pack-gm")]
fn run_pack_gm(w: &mut World) {
    let pack = w.pack.as_ref().unwrap().to_str().unwrap().to_string();
    w.outputs
        .push(support::spook(&["run", &pack, "--mcp", "pack-gm"]));
}

#[then(expr = "the pack-gm command fails")]
fn pack_gm_fails(w: &mut World) {
    let output = w.outputs.last().unwrap();
    assert!(
        !output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[then(expr = "the pack-gm stderr is NOT_IN_SPEC")]
fn pack_gm_stderr(w: &mut World) {
    let output = w.outputs.last().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&output.stderr).trim(),
        "NOT_IN_SPEC"
    );
}
