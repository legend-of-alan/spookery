use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Child;

use cucumber::writer::Stats;
use cucumber::World as _;

/// An HTTP GM held across steps. Dropped with the scenario.
pub struct Held(
    pub rmcp::service::RunningService<rmcp::RoleClient, rmcp::model::ClientConfig>,
);

impl std::fmt::Debug for Held {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Held").finish_non_exhaustive()
    }
}

#[derive(Debug, Default, cucumber::World)]
pub struct World {
    pub pack: Option<PathBuf>,
    pub scratch: String,
    pub before: Vec<(PathBuf, Vec<u8>)>,
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub mark: usize,
    pub player_added: String,
    pub now_bytes: Vec<u8>,
    pub play: Option<support::Proc>,
    pub plays: Vec<support::Proc>,
    pub gm: Option<support::Proc>,
    pub summary: Vec<String>,
    pub seen: Vec<(String, Vec<String>)>,
    pub told: Vec<(String, String)>,
    pub live: Option<table::Live>,
    pub children: Vec<Child>,
    pub listeners: Vec<TcpListener>,
    pub seated: Vec<(PathBuf, spookery_kernel::api::model::Presence)>,
    pub holder: Option<Held>,
}

fn scenario_limit() -> usize {
    // One live server per folder. More than two at once pushes the existing
    // wall-clock polls over: five seconds for a fast-forward, two seconds
    // for a second player's line.
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(2)
}

fn finish(world: &mut World) {
    world.holder.take();
    drop(world.live.take());
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

async fn run_dir(dir: &'static str) -> bool {
    // Folders run together, one scenario at a time inside each folder. A step
    // blocks its runtime, so extra scenarios in a folder only pile up servers.
    let writer = World::cucumber()
        .max_concurrent_scenarios(1)
        .fail_on_skipped()
        .after(|_, _, _, _, world| {
            if let Some(world) = world {
                finish(world);
            }
            Box::pin(async {})
        })
        .run(dir)
        .await;
    writer.execution_has_failed()
}

fn main() {
    let limit = scenario_limit();
    let dirs: Vec<&'static str> = feature_dirs().to_vec();
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
