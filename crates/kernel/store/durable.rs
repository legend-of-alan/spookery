//! Flush a file or a directory, and rename into place.

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

use crate::error::Reason;

pub(crate) fn tmp_path(target: &Path) -> PathBuf {
    let mut name = target.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    target.with_file_name(name)
}

pub(crate) fn fsync_file(path: &Path) -> Result<(), Reason> {
    if refuse_sync(path) {
        return Err(Reason::CommitConflict);
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|_| Reason::CommitConflict)?;
    file.sync_all().map_err(|_| Reason::CommitConflict)?;
    note_file(path);
    Ok(())
}

#[cfg(test)]
fn refuse_sync(path: &Path) -> bool {
    trace::take_fail(path)
}

#[cfg(not(test))]
fn refuse_sync(_: &Path) -> bool {
    false
}

pub(crate) fn fsync_dir(path: &Path) -> Result<(), Reason> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)
            .map_err(|_| Reason::CommitConflict)?;
        file.sync_all().map_err(|_| Reason::CommitConflict)?;
        note_dir(path);
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let file = std::fs::File::open(path).map_err(|_| Reason::CommitConflict)?;
        file.sync_all().map_err(|_| Reason::CommitConflict)?;
        note_dir(path);
        Ok(())
    }
}

pub(crate) fn rename(from: &Path, to: &Path) -> Result<(), Reason> {
    std::fs::rename(from, to).map_err(|_| Reason::CommitConflict)?;
    note_rename(to);
    Ok(())
}

#[cfg(test)]
fn note_file(path: &Path) {
    trace::record(Mark::File(path.to_path_buf()));
}

#[cfg(not(test))]
fn note_file(_: &Path) {}

#[cfg(test)]
fn note_dir(path: &Path) {
    trace::record(Mark::Dir(path.to_path_buf()));
}

#[cfg(not(test))]
fn note_dir(_: &Path) {}

#[cfg(test)]
fn note_rename(path: &Path) {
    trace::record(Mark::Rename(path.to_path_buf()));
}

#[cfg(not(test))]
fn note_rename(_: &Path) {}

#[cfg(test)]
pub(crate) use trace::{clear_fail, fail_next, start as start_trace, take as take_trace, Mark};

#[cfg(test)]
mod trace {
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(crate) enum Mark {
        File(PathBuf),
        Dir(PathBuf),
        Rename(PathBuf),
    }

    thread_local! {
        static LOG: RefCell<Option<Vec<Mark>>> = const { RefCell::new(None) };
        static FAIL: RefCell<Option<&'static str>> = const { RefCell::new(None) };
    }

    pub(crate) fn fail_next(suffix: &'static str) {
        FAIL.with(|fail| *fail.borrow_mut() = Some(suffix));
    }

    pub(crate) fn clear_fail() {
        FAIL.with(|fail| *fail.borrow_mut() = None);
    }

    pub(super) fn take_fail(path: &Path) -> bool {
        FAIL.with(|fail| {
            let mut slot = fail.borrow_mut();
            let Some(suffix) = *slot else {
                return false;
            };
            let hit = path
                .file_name()
                .and_then(|file| file.to_str())
                .is_some_and(|name| name.ends_with(suffix));
            if hit {
                *slot = None;
            }
            hit
        })
    }

    pub(super) fn record(mark: Mark) {
        LOG.with(|log| {
            if let Some(marks) = log.borrow_mut().as_mut() {
                marks.push(mark);
            }
        });
    }

    pub(crate) fn start() {
        LOG.with(|log| *log.borrow_mut() = Some(Vec::new()));
        clear_fail();
    }

    pub(crate) fn take() -> Vec<Mark> {
        LOG.with(|log| log.borrow_mut().take().unwrap_or_default())
    }
}
