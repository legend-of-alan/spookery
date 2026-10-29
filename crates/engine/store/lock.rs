//! Lock the pack for one writer, and keep readers out while it writes.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::Path;

use crate::error::Reason;
use crate::store::recover::recover;

/// Held for as long as a process has the pack open for writing.
/// Dropping it closes the file, which releases the lock.
pub struct LockGuard {
    _file: File,
}

/// Held while the writer publishes, so no reader sees a write halfway done.
/// The inner write helpers take `&WriteGuard`, so a write without one does not compile:
///
/// ```compile_fail
/// # let root = std::path::Path::new("pack");
/// spookery_engine::testing::append_bundle(root, "{}\n");
/// ```
///
/// ```no_run
/// # let root = std::path::Path::new("pack");
/// let _lock = spookery_engine::lock_pack(root).unwrap();
/// let write = spookery_engine::testing::write_guard(root).unwrap();
/// spookery_engine::testing::append_bundle(root, "{}\n", &write).unwrap();
/// ```
pub struct WriteGuard {
    _file: File,
}

/// Held while a reader builds a World from the files on disk.
pub struct ReadGuard {
    _file: File,
}

/// Take `state/.pack.lock`, then finish or roll back a commit a crash left half done.
/// The lock makes this process the pack's writer, so only it repairs the pack.
/// A path with no `state/` is `NOT_IN_SPEC`. A second writer is refused with `COMMIT_CONFLICT`.
pub fn lock_pack(root: &Path) -> Result<LockGuard, Reason> {
    let file = open_lock(root, "state/.pack.lock")?;
    file.try_lock().map_err(|_| Reason::CommitConflict)?;
    recover(root, &write_guard(root)?)?;
    Ok(LockGuard { _file: file })
}

/// Wait for readers to finish, then keep new ones out until the guard drops.
/// Only the pack's writer takes it, and never inside another write guard.
pub fn write_guard(root: &Path) -> Result<WriteGuard, Reason> {
    let file = open_lock(root, "state/.pack.rw.lock")?;
    file.lock().map_err(|_| Reason::CommitConflict)?;
    Ok(WriteGuard { _file: file })
}

/// Wait out a write in progress, then hold the next one off until the guard drops.
/// An existing lock file opens read-only, so a reader needs no write access to `state/`.
pub fn read_guard(root: &Path) -> Result<ReadGuard, Reason> {
    let file = match File::open(root.join("state/.pack.rw.lock")) {
        Ok(file) => file,
        Err(_) => open_lock(root, "state/.pack.rw.lock")?,
    };
    match file.try_lock_shared() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => {
            #[cfg(feature = "testing")]
            note_reader_wait();
            file.lock_shared().map_err(|_| Reason::NotInSpec)?;
        }
        Err(TryLockError::Error(_)) => return Err(Reason::NotInSpec),
    }
    Ok(ReadGuard { _file: file })
}

fn open_lock(root: &Path, rel: &str) -> Result<File, Reason> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join(rel))
        .map_err(|_| Reason::NotInSpec)
}

#[cfg(feature = "testing")]
static READER_WAIT: std::sync::Mutex<Option<std::sync::mpsc::Sender<&'static str>>> =
    std::sync::Mutex::new(None);

/// Send "waited" to `tx` the next time a reader waits out a write.
#[cfg(feature = "testing")]
pub fn on_reader_wait(tx: std::sync::mpsc::Sender<&'static str>) {
    *READER_WAIT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(tx);
}

#[cfg(feature = "testing")]
fn note_reader_wait() {
    let tx = READER_WAIT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    if let Some(tx) = tx {
        let _ = tx.send("waited");
    }
}
