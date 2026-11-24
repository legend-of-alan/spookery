use spookery_engine::component;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Child;

use cucumber::writer::Stats;
use cucumber::World as _;

#[derive(Default, cucumber::World)]
pub struct World {
    pub pack: Option<PathBuf>,
    pub scratch: String,
    pub before: Vec<(PathBuf, Vec<u8>)>,
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub mark: usize,
    pub host_mark: usize,
    pub wren_mark: usize,
    pub player_added: String,
    /// The `claimed` reply of a pick, before any claim by token.
    pub pc_picked_reply: serde_json::Value,
    pub silent_closed: bool,
    pub flood_paced: usize,
    pub flood_refilled: usize,
    pub flood_other_at: usize,
    pub now_bytes: Vec<u8>,
    pub state_files: Vec<(PathBuf, Vec<u8>)>,
    pub(crate) play: Option<support::Process>,
    #[allow(dead_code, reason = "a drop guard; only features_host spawns these plays")]
    pub(crate) plays: Vec<support::Process>,
    #[allow(dead_code, reason = "only features_host runs the gm terminal")]
    pub(crate) gm: Option<support::Process>,
    pub summary: Vec<String>,
    pub seen: Vec<(String, Vec<String>)>,
    pub told: Vec<(String, String)>,
    pub live: Option<host::Host>,
    pub children: Vec<Child>,
    pub listeners: Vec<TcpListener>,
    pub seated: Vec<(PathBuf, spookery_engine::Presence)>,
    pub(crate) http_gm: Option<mcp_client::GmClient>,
    /// An MCP session opened over plain HTTP, and its event streams.
    pub(crate) mcp_session: Option<mcp_client::RawSession>,
    /// The host the WebSocket GM connects to, beside `live`, and the pack copy it runs.
    pub ws_host: Option<(host::Process, PathBuf)>,
    /// A recorded session's lines, each one message a seat sent or got.
    pub recorded: Vec<serde_json::Value>,
    /// The seats a recorded session was replayed on, by the name the recording gives each.
    pub replayed: Vec<(String, host::Seat)>,
    /// This build's own recording of the replayed session.
    pub replay_recording_path: PathBuf,
    /// The scripted model a scenario's AI GM plays with.
    pub model: scripted_gm::ScriptedModel,
    /// The AI GM once it connects, run in this process.
    pub ai_gm: Option<scripted_gm::AiGm>,
    /// What a scenario's HTTP model stub has read.
    pub stub_seen: Option<std::sync::Arc<std::sync::Mutex<scripted_gm::Seen>>>,
    /// World time before a scenario's player acts.
    pub start_ticks: u64,
    /// The world times of the rounds a scenario's player heard.
    pub round_ticks: Vec<u64>,
}

impl std::fmt::Debug for World {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("World").finish_non_exhaustive()
    }
}

#[cucumber::given(expr = "a docks table where Bram is called {string} and stands in the Gate Yard")]
fn a_docks_table_where_bram_is_called(world: &mut World, name: String) {
    let pack = support::copy_pack("docks", "bram-renamed");
    let bram = pack.join("cast/bram.md");
    let text = std::fs::read_to_string(&bram).unwrap_or_else(|err| panic!("bram: {err}"));
    let renamed = text.replace("name: Bram", &format!("name: {name}"));
    std::fs::write(&bram, renamed).unwrap_or_else(|err| panic!("bram: {err}"));
    spookery_engine::testing::edit_save(&pack, |now| {
        let mut actors = now.actors_mut();
        actors.get_mut::<component::Place>(&"bram".into()).set("gate_yard").unwrap();
    });
    world.pack = Some(pack);
}

#[cucumber::given(expr = "a docks table where the shop, Aldric and the lamp oil have no name")]
fn a_docks_table_where_three_have_no_name(world: &mut World) {
    let pack = support::copy_pack("docks", "no-names");
    for (file, line) in [
        ("world/places/docks_shop.md", "name: Docks Shop\n"),
        ("cast/aldric.md", "name: Aldric\n"),
        ("world/items/lamp_oil.md", "name: Lamp oil\n"),
    ] {
        let path = pack.join(file);
        let text = std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{file}: {err}"));
        assert!(text.contains(line), "{file} has no {line}");
        std::fs::write(&path, text.replace(line, "")).unwrap_or_else(|err| panic!("{file}: {err}"));
    }
    spookery_engine::testing::stamp_pack_rev(&pack);
    world.pack = Some(pack);
}

/// The folders under `tests/features` the host program runs. The world program runs everything else there.
const HOST: &[&str] = &[
    "cli", "digest", "host", "limits", "loopback", "mcp", "new", "seats", "sessions", "speech",
];

/// The folders and loose feature files under `tests/features` this program runs.
fn feature_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir("tests/features")
        .unwrap_or_else(|err| panic!("read tests/features: {err}"))
        .map(|entry| entry.unwrap_or_else(|err| panic!("read tests/features: {err}")).path())
        .filter(|path| path.is_dir() || path.extension().is_some_and(|ext| ext == "feature"))
        .filter(|path| runs(&path.file_name().unwrap().to_string_lossy()))
        .collect();
    dirs.sort();
    dirs
}

fn scenario_limit() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(8)
}

fn tear_down(world: &mut World) {
    world.http_gm.take();
    world.mcp_session.take();
    drop(world.live.take());
    world.ai_gm.take();
    if let Some((host, pack)) = world.ws_host.take() {
        drop(host);
        let _ = std::fs::remove_dir_all(pack);
    }
    world.listeners.clear();
    world.seated.clear();
    for mut child in world.children.drain(..) {
        let _ = child.kill();
        let _ = child.wait();
    }
    if let Some(pack) = world.pack.take() {
        let _ = std::fs::remove_dir_all(pack);
    }
}

async fn run_dir(dir: &Path) -> bool {
    let writer = World::cucumber()
        .max_concurrent_scenarios(4)
        .fail_on_skipped()
        .after(|_, _, _, _, world| {
            if let Some(world) = world {
                tear_down(world);
            }
            Box::pin(async {})
        })
        .run(dir)
        .await;
    writer.execution_has_failed()
}

fn main() {
    let limit = scenario_limit();
    let dirs = feature_dirs();
    let failed = std::sync::atomic::AtomicBool::new(false);
    let next = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|scope| {
        let workers = limit.min(dirs.len().max(1));
        for _ in 0..workers {
            scope.spawn(|| {
                let runtime = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(4)
                    .enable_all()
                    .build()
                    .unwrap_or_else(|err| panic!("runtime: {err}"));
                loop {
                    let index = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(dir) = dirs.get(index) else {
                        break;
                    };
                    if runtime.block_on(run_dir(dir)) {
                        failed.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            });
        }
    });
    if failed.load(std::sync::atomic::Ordering::Relaxed) {
        std::process::exit(1);
    }
}
