//! Lock the pack for one writer.

use std::fs::{File, OpenOptions};
use std::path::Path;

use crate::error::Reason;
use crate::store::recover::recover;

/// Held for as long as a process has the pack open for writing.
/// Dropping it closes the file, which releases the lock.
pub struct LockGuard {
    _file: File,
}

/// Take `state/.pack.lock`, then finish or roll back a commit a crash left half done.
/// The lock makes this process the pack's writer, so only it repairs the pack.
/// A path with no `state/` is `NOT_IN_SPEC`. A second writer is refused with `COMMIT_CONFLICT`.
pub fn lock_pack(root: &Path) -> Result<LockGuard, Reason> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join("state/.pack.lock"))
        .map_err(|_| Reason::NotInSpec)?;
    file.try_lock().map_err(|_| Reason::CommitConflict)?;
    recover(root)?;
    Ok(LockGuard { _file: file })
}
