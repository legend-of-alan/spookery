use std::process::Child;
use std::sync::Mutex;

use cucumber::writer::Stats;
use cucumber::World as _;

static CHILDREN: Mutex<Vec<Child>> = Mutex::new(Vec::new());
static LIVE: Mutex<Option<table::Live>> = Mutex::new(None);

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|err| err.into_inner())
}

/// Scenarios that spawn `spook` outside [`table::Live`] register the child here.
/// The after hook drops every child, including when the scenario fails.
#[allow(dead_code)]
pub fn track_child(child: Child) {
    lock(&CHILDREN).push(child);
}

pub fn set_live(live: table::Live) {
    if let Some(old) = lock(&LIVE).replace(live) {
        drop(old);
    }
}

pub fn with_live<T>(body: impl FnOnce(&mut table::Live) -> T) -> T {
    let mut slot = lock(&LIVE);
    let live = slot
        .as_mut()
        .unwrap_or_else(|| panic!("no table is listening"));
    body(live)
}

fn drop_runtime() {
    finish_scenario();
    drop(lock(&LIVE).take());
    for mut child in lock(&CHILDREN).drain(..) {
        let _ = child.kill();
        let _ = child.wait();
    }
}

#[derive(Debug, Default, cucumber::World)]
pub struct World {
    pub pack: Option<std::path::PathBuf>,
    pub scratch: String,
    pub runner: String,
    pub before: Vec<(std::path::PathBuf, Vec<u8>)>,
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
}

#[tokio::main]
async fn main() {
    let mut failed = false;
    for dir in feature_dirs() {
        let writer = World::cucumber()
            .max_concurrent_scenarios(1)
            .fail_on_skipped()
            .after(|_, _, _, _, world| {
                drop_runtime();
                if let Some(world) = world {
                    if let Some(pack) = world.pack.take() {
                        let _ = std::fs::remove_dir_all(pack);
                    }
                }
                Box::pin(async {})
            })
            .run(dir)
            .await;
        failed |= writer.execution_has_failed();
    }
    if failed {
        std::process::exit(1);
    }
}
