use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rmcp::service::ServiceExt;
use rmcp::transport::TokioChildProcess;
use tokio::process::Command;

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

#[tokio::test]
async fn product_01_load_mcp() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixture = manifest.join("packs/oneshot-docks");
    let pack = std::env::temp_dir().join(format!(
        "spook-product01-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&pack);
    copy_dir(&fixture, &pack);

    let mut command = Command::new(env!("CARGO_BIN_EXE_spook"));
    command.args(["run", pack.to_str().unwrap(), "--mcp", "pack-table"]);
    let running = ().serve(
        TokioChildProcess::new(command).unwrap_or_else(|err| panic!("spawn spook: {err}")),
    );
    let running = tokio::time::timeout(Duration::from_secs(15), running)
        .await
        .unwrap_or_else(|_| panic!("initialize timed out"))
        .unwrap_or_else(|err| panic!("initialize: {err}"));

    let listed = tokio::time::timeout(Duration::from_secs(15), running.list_tools(None))
        .await
        .unwrap_or_else(|_| panic!("tools/list timed out"))
        .unwrap_or_else(|err| panic!("tools/list: {err}"));
    let names: Vec<&str> = listed.tools.iter().map(|tool| tool.name.as_ref()).collect();
    assert_eq!(names, TABLE_TOOLS);

    let _ = fs::remove_dir_all(&pack);
}

const TICK_TOOLS: &[&str] = &[
    "session_status",
    "perceive",
    "recall",
    "npc_act",
    "resolve_check",
    "apply_effect",
    "tick_commit",
];

const STUDIO_TOOLS: &[&str] = &[
    "pack_info",
    "read_entity",
    "patch_entity",
    "lint_pack",
    "simulate_knowledge",
    "run_conformance",
    "compile_preview",
];

fn owned(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_string()).collect()
}

async fn listed_names(pack: &Path, profile: &str) -> Vec<String> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_spook"));
    command.args(["run", pack.to_str().unwrap(), "--mcp", profile]);
    let running = ().serve(
        TokioChildProcess::new(command).unwrap_or_else(|err| panic!("spawn spook: {err}")),
    );
    let running = tokio::time::timeout(Duration::from_secs(15), running)
        .await
        .unwrap_or_else(|_| panic!("{profile} initialize timed out"))
        .unwrap_or_else(|err| panic!("{profile} initialize: {err}"));
    let listed = tokio::time::timeout(Duration::from_secs(15), running.list_tools(None))
        .await
        .unwrap_or_else(|_| panic!("{profile} tools/list timed out"))
        .unwrap_or_else(|err| panic!("{profile} tools/list: {err}"));
    listed
        .tools
        .iter()
        .map(|tool| tool.name.to_string())
        .collect()
}

#[tokio::test]
async fn pack_tick_and_studio_tools_follow_section_8() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixture = manifest.join("packs/oneshot-docks");
    let pack = std::env::temp_dir().join(format!(
        "spook-profiles-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&pack);
    copy_dir(&fixture, &pack);

    let tick = listed_names(&pack, "pack-tick").await;
    assert_eq!(tick, owned(TICK_TOOLS));
    let studio = listed_names(&pack, "pack-studio").await;
    assert_eq!(studio, owned(STUDIO_TOOLS));

    let refused = std::process::Command::new(env!("CARGO_BIN_EXE_spook"))
        .args(["run", pack.to_str().unwrap(), "--mcp", "pack-gm"])
        .output()
        .unwrap_or_else(|err| panic!("spawn spook: {err}"));
    assert!(!refused.status.success());
    assert_eq!(
        String::from_utf8_lossy(&refused.stderr).trim(),
        "NOT_IN_SPEC"
    );

    let _ = fs::remove_dir_all(&pack);
}
