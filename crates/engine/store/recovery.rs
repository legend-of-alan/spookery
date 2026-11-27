//! Cut a torn append, complete a checkpoint a crash left half done, and remove stray temp files.

use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};

use crate::store::durability::{add_suffix, fsync_dir, fsync_file, refuse_io, rename};
use crate::store::journal;
use crate::store::lock::WriteGuard;
use crate::Reason;

/// Run by `lock::take` before the writer reads a pack that a crash may have left mid-commit.
pub(crate) fn recover(root: &Path, write: &WriteGuard) -> Result<(), Reason> {
    let state = root.join("state");
    cut_torn_end(root)?;
    close_checkpointed(root, write)?;
    remove_tmp(&state);
    Ok(())
}

/// Gzip every uncompressed bundle that ends in a published checkpoint.
/// When the gzip is already complete, only the uncompressed file is removed. A bundle whose
/// snapshot is missing stays open, and the next commit writes the snapshot and closes it.
pub(crate) fn close_checkpointed(root: &Path, _write: &WriteGuard) -> Result<(), Reason> {
    let mut failed = Ok(());
    for bundle in list_uncompressed(root) {
        if journal::trailing_checkpoint(&bundle).is_none() {
            continue;
        }
        if let Err(reason) = close_one(root, &bundle) {
            failed = Err(reason);
        }
    }
    failed
}

fn close_one(root: &Path, bundle: &Path) -> Result<(), Reason> {
    let bytes = fs::read(bundle).map_err(refuse_io)?;
    let gz = add_suffix(bundle, ".gz");
    if gz_matches(&gz, &bytes) {
        return remove_uncompressed(bundle);
    }
    if !journal::is_published(root, bundle) || !snapshot_matches(root, bundle) {
        return Ok(());
    }
    let tmp = add_suffix(&gz, ".tmp");
    let gz_bytes = journal::gzip_bytes(&bytes).map_err(refuse_io)?;
    fs::write(&tmp, gz_bytes).map_err(refuse_io)?;
    fsync_file(&tmp)?;
    rename(&tmp, &gz)?;
    remove_uncompressed(bundle)
}

fn snapshot_matches(root: &Path, bundle: &Path) -> bool {
    let Some(mark) = journal::trailing_checkpoint(bundle) else {
        return false;
    };
    let snap = journal::checkpoint_path(root, mark.state_rev);
    journal::hashes_to(&snap, &mark.state_hash)
}

fn gz_matches(gz: &Path, uncompressed: &[u8]) -> bool {
    if !gz.is_file() {
        return false;
    }
    let Ok(bytes) = fs::read(gz) else {
        return false;
    };
    journal::gunzip_bytes(&bytes).ok().as_deref() == Some(uncompressed)
}

fn remove_uncompressed(bundle: &Path) -> Result<(), Reason> {
    fs::remove_file(bundle).map_err(refuse_io)?;
    if let Some(parent) = bundle.parent() {
        fsync_dir(parent)?;
    }
    Ok(())
}

fn list_uncompressed(root: &Path) -> Vec<PathBuf> {
    let dir = root.join("state/journal");
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut bundles = Vec::new();
    for entry in entries.flatten() {
        let file = entry.file_name();
        let Some(file) = file.to_str() else {
            continue;
        };
        if file.ends_with(".jsonl") && journal::bundle_index(file).is_some() {
            bundles.push(entry.path());
        }
    }
    bundles.sort();
    bundles
}

fn cut_torn_end(root: &Path) -> Result<(), Reason> {
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
    tracing::warn!(
        bundle_path = %path.display(),
        cut_bytes = bytes.len() - end,
        "cut a torn journal record a crash left"
    );
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
            .and_then(|file| file.to_str())
            .is_some_and(|file| file.ends_with(".tmp"))
        {
            let _ = fs::remove_file(path);
        }
    }
}
