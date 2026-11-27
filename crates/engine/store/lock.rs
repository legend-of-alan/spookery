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
            match file.try_lock() {
                Ok(()) => Ok(Self(file)),
                Err(TryLockError::WouldBlock) => Err(Reason::CommitConflict),
                Err(TryLockError::Error(_)) => Err(Reason::StoreIo),
            }
        }

        /// Wait for the exclusive lock.
        pub(super) fn lock(file: File) -> Result<Self, Reason> {
            file.lock().map_err(|_| Reason::NotInSpec)?;
            Ok(Self(file))
        }

        /// Take a shared lock, calling `on_wait` first when a writer holds the file.
        pub(super) fn lock_shared(file: File, on_wait: impl FnOnce()) -> Result<Self, Reason> {
            match file.try_lock_shared() {
                Ok(()) => return Ok(Self(file)),
                Err(TryLockError::WouldBlock) => on_wait(),
                Err(TryLockError::Error(_)) => return Err(Reason::NotInSpec),
            }
            file.lock_shared().map_err(|_| Reason::NotInSpec)?;
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
/// The inner write helpers take `&WriteGuard`, so a write without one does not compile.
#[cfg_attr(not(feature = "testing"), expect(unreachable_pub))]
pub struct WriteGuard {
    _file: LockedFile,
}

/// Held while a reader builds a session from the files on disk.
pub(crate) struct ReadGuard {
    _file: LockedFile,
}

/// Take `state/.pack.lock`, then complete or roll back a commit a crash left half done.
/// The lock makes this process the pack's writer, so only it repairs the pack.
/// A path with no `state/` is `NOT_IN_SPEC`. A second writer is refused with `COMMIT_CONFLICT`.
pub fn take(root: &Path) -> Result<PackGuard, Reason> {
    let file = open(root, "state/.pack.lock")?;
    let lock = PackGuard {
        _file: LockedFile::try_lock(file)?,
    };
    recover(root, &take_write(root, &lock)?)?;
    Ok(lock)
}

/// Wait for readers to be done, then keep new ones out until the guard drops.
/// Only the pack's writer takes it, and never inside another write guard.
pub(crate) fn take_write(root: &Path, _lock: &PackGuard) -> Result<WriteGuard, Reason> {
    Ok(WriteGuard {
        _file: LockedFile::lock(open(root, RW_LOCK)?)?,
    })
}

/// Wait out a write in progress, then hold the next one off until the guard drops.
/// An existing lock file opens read-only, so a reader needs no write access to `state/`.
pub(crate) fn take_read(root: &Path) -> Result<ReadGuard, Reason> {
    let file = match File::open(root.join(RW_LOCK)) {
        Ok(file) => file,
        Err(_) => open(root, RW_LOCK)?,
    };
    Ok(ReadGuard {
        _file: LockedFile::lock_shared(file, || {
            #[cfg(feature = "testing")]
            {
                let mut handlers = READ_WAIT_HANDLERS.lock().unwrap();
                if let Some(index) = handlers.iter().position(|(watched, _)| watched == root) {
                    let (_, handler) = handlers.swap_remove(index);
                    drop(handlers);
                    handler();
                }
            }
        })?,
    })
}

#[cfg(feature = "testing")]
type ReadWaitHandler = Box<dyn FnOnce() + Send>;

/// Handlers a test set with `set_read_wait_handler`, each with the pack root it watches.
#[cfg(feature = "testing")]
static READ_WAIT_HANDLERS: std::sync::Mutex<Vec<(std::path::PathBuf, ReadWaitHandler)>> =
    std::sync::Mutex::new(Vec::new());

/// Call `handler` once, the next time a read of `root` finds the write guard held,
/// just before it waits for the guard to drop.
#[cfg(feature = "testing")]
pub fn set_read_wait_handler(root: &Path, handler: impl FnOnce() + Send + 'static) {
    let mut handlers = READ_WAIT_HANDLERS.lock().unwrap();
    handlers.push((root.to_path_buf(), Box::new(handler)));
}

fn open(root: &Path, rel: &str) -> Result<File, Reason> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join(rel))
        .map_err(|_| Reason::NotInSpec)
}
