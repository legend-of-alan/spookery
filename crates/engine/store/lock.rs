//! Lock the pack for one writer, and keep readers out while it writes.

use std::fs::{File, OpenOptions};
use std::path::Path;

use crate::store::recovery::recover;
use crate::Reason;

const RW_LOCK: &str = "state/.pack.rw.lock";

use locked::LockedFile;

mod locked {
    use std::fs::{File, TryLockError};

    use crate::Reason;

    /// A locked file that unlocks on drop. Closing alone is not enough, since a child
    /// spawned by another thread holds a copy of the handle until it execs.
    /// The field is private to this module, so the only way to build one is a
    /// constructor below that takes the lock first.
    pub(super) struct LockedFile(File);

    impl LockedFile {
        /// Lock without waiting. A lock held elsewhere is `COMMIT_CONFLICT`.
        pub(super) fn try_lock(file: File) -> Result<Self, Reason> {
            file.try_lock().map_err(|_| Reason::CommitConflict)?;
            Ok(Self(file))
        }

        /// Wait for the exclusive lock.
        pub(super) fn lock(file: File) -> Result<Self, Reason> {
            file.lock().map_err(|_| Reason::NotInSpec)?;
            Ok(Self(file))
        }

        /// Wait for a shared lock, noting the wait for tests.
        pub(super) fn lock_shared(file: File) -> Result<Self, Reason> {
            match file.try_lock_shared() {
                Ok(()) => {}
                Err(TryLockError::WouldBlock) => {
                    #[cfg(feature = "testing")]
                    super::note_reader_wait();
                    file.lock_shared().map_err(|_| Reason::NotInSpec)?;
                }
                Err(TryLockError::Error(_)) => return Err(Reason::NotInSpec),
            }
            Ok(Self(file))
        }

        #[cfg(feature = "testing")]
        pub(super) fn file(&self) -> &File {
            &self.0
        }
    }

    impl Drop for LockedFile {
        fn drop(&mut self) {
            let _ = self.0.unlock();
        }
    }
}

/// Held for as long as a process has the pack open for writing.
/// Dropping it unlocks the file, which releases the lock.
pub struct PackGuard {
    _file: LockedFile,
}

#[cfg(feature = "testing")]
impl PackGuard {
    pub(crate) fn file(&self) -> &File {
        self._file.file()
    }
}

/// Held while the writer publishes, so no reader sees a write halfway done.
/// The inner write helpers take `&WriteGuard`, so a write without one does not compile:
///
/// ```compile_fail
/// # let root = std::path::Path::new("pack");
/// spookery_engine::testing::append_to_bundle(root, "{}\n");
/// ```
///
/// ```no_run
/// # let root = std::path::Path::new("pack");
/// let world = spookery_engine::testing::resume(root);
/// let write = spookery_engine::testing::lock_write(&world).unwrap();
/// spookery_engine::testing::append_to_bundle(root, "{}\n", &write).unwrap();
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
pub fn lock_pack(root: &Path) -> Result<PackGuard, Reason> {
    let file = open_lock(root, "state/.pack.lock")?;
    let lock = PackGuard {
        _file: LockedFile::try_lock(file)?,
    };
    recover(root, &lock_write(root, &lock)?)?;
    Ok(lock)
}

/// Wait for readers to finish, then keep new ones out until the guard drops.
/// Only the pack's writer takes it, and never inside another write guard.
pub fn lock_write(root: &Path, _lock: &PackGuard) -> Result<WriteGuard, Reason> {
    Ok(WriteGuard {
        _file: LockedFile::lock(open_lock(root, RW_LOCK)?)?,
    })
}

/// Wait out a write in progress, then hold the next one off until the guard drops.
/// An existing lock file opens read-only, so a reader needs no write access to `state/`.
pub fn lock_read(root: &Path) -> Result<ReadGuard, Reason> {
    let file = match File::open(root.join(RW_LOCK)) {
        Ok(file) => file,
        Err(_) => open_lock(root, RW_LOCK)?,
    };
    Ok(ReadGuard {
        _file: LockedFile::lock_shared(file)?,
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
pub fn watch_reader_wait(tx: std::sync::mpsc::Sender<&'static str>) {
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
