#[path = "steps/changes.rs"]
mod changes;
#[path = "steps/docks.rs"]
mod docks;
#[path = "steps/doors.rs"]
mod doors;
#[path = "steps/map.rs"]
mod map;
#[path = "steps/no_facts.rs"]
mod no_facts;
#[path = "steps/read_lock.rs"]
mod read_lock;
#[path = "steps/round_play.rs"]
mod round_play;

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
    pub summary_lines: Vec<String>,
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
        self.pack
            .as_deref()
            .unwrap_or_else(|| panic!("no docks table"))
    }
}

/// The first word of `line`, and the rest of it when there is any.
pub fn split_line(line: &str) -> (&str, Option<&str>) {
    line.split_once(' ')
        .map_or((line, None), |(head, rest)| (head, Some(rest)))
}

#[tokio::main]
async fn main() {
    let failed = World::cucumber()
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
        .run("tests/features")
        .await
        .execution_has_failed();
    if failed {
        std::process::exit(1);
    }
}
