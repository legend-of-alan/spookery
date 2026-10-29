//! Lock the pack for one writer, and keep readers out while it writes.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::Path;

use crate::error::Reason;
use crate::store::recover::recover;

const RW_LOCK: &str = "state/.pack.rw.lock";

/// A locked file that unlocks on drop. Closing alone is not enough, since a child
/// spawned by another thread holds a copy of the handle until it execs.
struct LockedFile(File);

impl Drop for LockedFile {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

/// Held for as long as a process has the pack open for writing.
/// Dropping it unlocks the file, which releases the lock.
pub struct LockGuard {
    _file: LockedFile,
}

#[cfg(feature = "testing")]
impl LockGuard {
    pub(crate) fn file(&self) -> &File {
        &self._file.0
    }
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
/// let world = spookery_engine::testing::open(root);
/// let write = spookery_engine::testing::write_guard(&world).unwrap();
/// spookery_engine::testing::append_bundle(root, "{}\n", &write).unwrap();
/// ```
pub struct WriteGuard {
    _file: LockedFile,
}

/// Held while a reader builds a World from the files on disk.
pub struct ReadGuard {
    _file: LockedFile,
}

/// Take `state/.pack.lock`, then finish or roll back a commit a crash left half done.
/// The lock makes this process the pack's writer, so only it repairs the pack.
/// A path with no `state/` is `NOT_IN_SPEC`. A second writer is refused with `COMMIT_CONFLICT`.
pub fn lock_pack(root: &Path) -> Result<LockGuard, Reason> {
    let file = open_lock(root, "state/.pack.lock")?;
    file.try_lock().map_err(|_| Reason::CommitConflict)?;
    let lock = LockGuard {
        _file: LockedFile(file),
    };
    recover(root, &write_guard(root, &lock)?)?;
    Ok(lock)
}

/// Wait for readers to finish, then keep new ones out until the guard drops.
/// Only the pack's writer takes it, and never inside another write guard.
pub fn write_guard(root: &Path, _lock: &LockGuard) -> Result<WriteGuard, Reason> {
    let file = open_lock(root, RW_LOCK)?;
    file.lock().map_err(|_| Reason::NotInSpec)?;
    Ok(WriteGuard {
        _file: LockedFile(file),
    })
}

/// Wait out a write in progress, then hold the next one off until the guard drops.
/// An existing lock file opens read-only, so a reader needs no write access to `state/`.
pub fn read_guard(root: &Path) -> Result<ReadGuard, Reason> {
    let file = match File::open(root.join(RW_LOCK)) {
        Ok(file) => file,
        Err(_) => open_lock(root, RW_LOCK)?,
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
    Ok(ReadGuard {
        _file: LockedFile(file),
    })
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
