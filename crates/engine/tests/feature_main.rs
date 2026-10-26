use std::path::{Path, PathBuf};

use cucumber::writer::Stats;
use cucumber::World as _;

#[derive(Debug, Default, cucumber::World)]
pub struct World {
    pub pack: Option<PathBuf>,
    pub scratch: String,
    pub stdout: String,
    pub mark: usize,
    pub player_added: String,
    pub now_bytes: Vec<u8>,
    pub summary: Vec<String>,
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
                if let Some(pack) = world.and_then(|w| w.pack.take()) {
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
