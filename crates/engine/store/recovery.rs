//! Cut a torn append, finish a checkpoint a crash left half done, and remove stray temp files.

use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};

use crate::store::durability::{add_suffix, fsync_dir, fsync_file, refuse_io, rename};
use crate::store::journal;
use crate::store::lock::WriteGuard;
use crate::Reason;

/// Run by `lock_pack` before the writer reads a pack that a crash may have left mid-commit.
pub fn recover(root: &Path, write: &WriteGuard) -> Result<(), Reason> {
    let state = root.join("state");
    cut_torn_tail(root)?;
    close_finished(root, write)?;
    remove_tmp(&state);
    Ok(())
}

/// Gzip every plain bundle that ends in a published checkpoint.
/// When the gzip is already complete, only the plain file is removed.
pub(crate) fn close_finished(root: &Path, _write: &WriteGuard) -> Result<(), Reason> {
    let mut failed = Ok(());
    for plain in plain_bundles(root) {
        if journal::trailing_checkpoint(&plain).is_none() {
            continue;
        }
        if let Err(reason) = close_one(root, &plain) {
            failed = Err(reason);
        }
    }
    failed
}

fn close_one(root: &Path, plain: &Path) -> Result<(), Reason> {
    let bytes = fs::read(plain).map_err(refuse_io)?;
    let gz = add_suffix(plain, ".gz");
    if gz_matches(&gz, &bytes) {
        return remove_plain(plain);
    }
    // Without its snapshot the bundle stays open, and the next commit's
    // settle step writes the snapshot and closes it.
    if !journal::is_published(root, plain) || !snapshot_matches(root, plain) {
        return Ok(());
    }
    let tmp = add_suffix(&gz, ".tmp");
    let gz_bytes = journal::gzip_bytes(&bytes).map_err(refuse_io)?;
    fs::write(&tmp, gz_bytes).map_err(refuse_io)?;
    fsync_file(&tmp)?;
    rename(&tmp, &gz)?;
    remove_plain(plain)
}

fn snapshot_matches(root: &Path, plain: &Path) -> bool {
    let Some(mark) = journal::trailing_checkpoint(plain) else {
        return false;
    };
    let snap = journal::checkpoint_path(root, mark.state_rev);
    journal::hashes_to(&snap, &mark.state_hash)
}

fn gz_matches(gz: &Path, plain: &[u8]) -> bool {
    if !gz.is_file() {
        return false;
    }
    let Ok(bytes) = fs::read(gz) else {
        return false;
    };
    journal::gunzip_bytes(&bytes).ok().as_deref() == Some(plain)
}

fn remove_plain(plain: &Path) -> Result<(), Reason> {
    fs::remove_file(plain).map_err(refuse_io)?;
    if let Some(parent) = plain.parent() {
        fsync_dir(parent)?;
    }
    Ok(())
}

fn plain_bundles(root: &Path) -> Vec<PathBuf> {
    let dir = root.join("state/journal");
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut plains = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.ends_with(".jsonl") && journal::bundle_index(name).is_some() {
            plains.push(entry.path());
        }
    }
    plains.sort();
    plains
}

fn cut_torn_tail(root: &Path) -> Result<(), Reason> {
    let Some(path) = journal::find_live_bundle(root) else {
        return Ok(());
    };
    let bytes = fs::read(&path).map_err(|_| Reason::Schema)?;
    if bytes.is_empty() || bytes.ends_with(b"\n") {
        return Ok(());
    }
    let end = bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map(|at| at + 1)
        .unwrap_or(0);
    let file = OpenOptions::new()
        .write(true)
        .open(&path)
        .map_err(refuse_io)?;
    file.set_len(end as u64).map_err(refuse_io)?;
    fsync_file(&path)?;
    Ok(())
}

fn remove_tmp(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            remove_tmp(&path);
        } else if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".tmp"))
        {
            let _ = fs::remove_file(path);
        }
    }
}
