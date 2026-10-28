//! Lock the pack for one writer.

use std::fs::{File, OpenOptions};
use std::path::Path;

use crate::error::Reason;

/// Held for as long as a process has the pack open for writing.
/// Dropping it closes the file, which releases the lock.
pub struct LockGuard {
    _file: File,
}

/// Take `state/.pack.lock`. A second writer is refused with `COMMIT_CONFLICT`.
pub fn lock_pack(root: &Path) -> Result<LockGuard, Reason> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join("state/.pack.lock"))
        .map_err(|_| Reason::CommitConflict)?;
    file.try_lock().map_err(|_| Reason::CommitConflict)?;
    Ok(LockGuard { _file: file })
}
