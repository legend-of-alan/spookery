use std::path::{Path, PathBuf};

use cucumber::writer::Stats;
use cucumber::World as _;

#[derive(Debug, Default, cucumber::World)]
pub struct World {
    pub pack: Option<PathBuf>,
    pub scratch: String,
    pub seen: String,
    pub count: usize,
    pub intent_id: String,
    pub oil: Vec<u8>,
    pub wait_order: Vec<String>,
    pub writer: Option<Writer>,
    pub lock_clone: Option<std::fs::File>,
}

/// The World a scenario holds open as the pack's writer. It owns the pack lock.
pub struct Writer {
    pub world: spookery_engine::World,
}

impl std::fmt::Debug for Writer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Writer")
    }
}

impl World {
    pub fn pack(&self) -> &Path {
        self.pack.as_deref().unwrap_or_else(|| panic!("no docks table"))
    }
}

#[tokio::main]
async fn main() {
    let mut failed = false;
    for &dir in feature_dirs() {
        failed |= World::cucumber()
            .max_concurrent_scenarios(1)
            .fail_on_skipped()
            .after(|_, _, _, _, world| {
                let pack = world.and_then(|w| {
                    w.writer = None;
                    w.lock_clone = None;
                    w.pack.take()
                });
                if let Some(pack) = pack {
                    let _ = std::fs::remove_dir_all(pack);
                }
                Box::pin(async {})
            })
            .run(dir)
            .await
            .execution_has_failed();
    }
    if failed {
        std::process::exit(1);
    }
}
