use std::fs;
use std::path::{Path, PathBuf};

use spookery_kernel::codec::{parse_recorded_line, RecordedLine};
use spookery_kernel::shard::open_shard;
use spookery_kernel::Reason;

use crate::run::{run_recorded_tick, RecordedTick};
use crate::seed::seed_preimage;

pub fn replay_recorded(source: &Path, snapshot: &Path) -> Result<PathBuf, Reason> {
    let temp = std::env::temp_dir().join(format!("spook-replay-{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp);
    copy_dir(snapshot, &temp)?;
    let mut lines = read_tick_lines(source)?;
    lines.sort_by_key(|line| line.tick_index);
    let mut shard = open_shard(&temp)?;
    for line in lines {
        if shard.now().tick_index + 1 != line.tick_index {
            return Err(Reason::Schema);
        }
        let seed = seed_preimage(&shard.now().shard_id, line.tick_index);
        if line.seed != seed {
            return Err(Reason::Schema);
        }
        let wall = line.last_tick_at.clone();
        let rev_after = line.state_rev_after;
        run_recorded_tick(
            &mut shard,
            &wall,
            RecordedTick {
                intents: line.intents,
                presence_humans: line.presence_humans,
            },
        )?;
        if shard.now().state_rev != rev_after {
            return Err(Reason::Schema);
        }
    }
    Ok(temp)
}

fn read_tick_lines(source: &Path) -> Result<Vec<RecordedLine>, Reason> {
    let dir = source.join("state/log");
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut lines = Vec::new();
    for entry in fs::read_dir(&dir).map_err(|_| Reason::Schema)? {
        let entry = entry.map_err(|_| Reason::Schema)?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !(name.starts_with("tick-") && name.ends_with(".jsonl")) {
            continue;
        }
        let text = fs::read_to_string(entry.path()).map_err(|_| Reason::Schema)?;
        lines.push(parse_recorded_line(&text)?);
    }
    Ok(lines)
}

fn copy_dir(from: &Path, to: &Path) -> Result<(), Reason> {
    fs::create_dir_all(to).map_err(|_| Reason::Schema)?;
    for entry in fs::read_dir(from).map_err(|_| Reason::Schema)? {
        let entry = entry.map_err(|_| Reason::Schema)?;
        let dest = to.join(entry.file_name());
        let file_type = entry.file_type().map_err(|_| Reason::Schema)?;
        if file_type.is_dir() {
            copy_dir(&entry.path(), &dest)?;
        } else {
            fs::copy(entry.path(), &dest).map_err(|_| Reason::Schema)?;
        }
    }
    Ok(())
}
