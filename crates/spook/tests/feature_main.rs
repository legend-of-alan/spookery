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
    pub silent_closed: bool,
    pub flood_paced: usize,
    pub flood_refilled: usize,
    pub flood_other_at: usize,
    pub now_bytes: Vec<u8>,
    pub state_files: Vec<(PathBuf, Vec<u8>)>,
    pub play: Option<support::Process>,
    pub plays: Vec<support::Process>,
    pub gm: Option<support::Process>,
    pub summary: Vec<String>,
    pub seen: Vec<(String, Vec<String>)>,
    pub told: Vec<(String, String)>,
    pub live: Option<table::Host>,
    pub children: Vec<Child>,
    pub listeners: Vec<TcpListener>,
    pub seated: Vec<(PathBuf, spookery_engine::Presence)>,
    pub http_gm: Option<rmcp::service::RunningService<rmcp::RoleClient, rmcp::model::ClientConfig>>,
    /// The table the WebSocket GM sits at, beside `live`, and the pack copy it runs.
    pub ws_table: Option<(table::RunningTable, PathBuf)>,
    /// A recorded session's lines, each one message a seat sent or got.
    pub recorded: Vec<serde_json::Value>,
    /// The seats a recorded session was replayed on, by the name the recording gives each.
    pub replayed: Vec<(String, table::Seat)>,
}

impl std::fmt::Debug for World {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("World").finish_non_exhaustive()
    }
}

#[cucumber::given(expr = "a copy of the sample-d20 pack")]
fn a_copy_of_the_sample_d20_pack(world: &mut World) {
    world.pack = Some(support::copy_pack("sample-d20", "sample"));
}

/// The folders under `tests/features` the host program runs. The world program runs every other folder.
const HOST: &[&str] = &[
    "cli", "digest", "host", "limits", "loopback", "mcp", "new", "seats", "sessions", "speech",
];

/// The folders under `tests/features` this program runs.
fn feature_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir("tests/features")
        .unwrap_or_else(|err| panic!("read tests/features: {err}"))
        .map(|entry| entry.unwrap_or_else(|err| panic!("read tests/features: {err}")).path())
        .filter(|path| path.is_dir() && runs(&path.file_name().unwrap().to_string_lossy()))
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
    drop(world.live.take());
    if let Some((table, pack)) = world.ws_table.take() {
        drop(table);
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
    // Folders run together, one scenario at a time inside each folder. A step
    // blocks its runtime, so extra scenarios in a folder only pile up servers.
    let writer = World::cucumber()
        .max_concurrent_scenarios(1)
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
                    .worker_threads(1)
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
