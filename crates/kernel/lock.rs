use std::fs::{File, OpenOptions};
use std::path::Path;

#[cfg(unix)]
use std::os::unix::io::AsRawFd;
#[cfg(windows)]
use std::os::windows::io::AsRawHandle;

use crate::error::Reason;
use crate::world::World;

#[cfg(unix)]
const LOCK_EX: i32 = 2;
#[cfg(unix)]
const LOCK_UN: i32 = 8;
#[cfg(unix)]
const LOCK_NB: i32 = 4;

#[cfg(unix)]
unsafe extern "C" {
    fn flock(fd: i32, op: i32) -> i32;
}

#[cfg(windows)]
const LOCKFILE_FAIL_IMMEDIATELY: u32 = 0x0000_0001;
#[cfg(windows)]
const LOCKFILE_EXCLUSIVE_LOCK: u32 = 0x0000_0002;

#[cfg(windows)]
#[repr(C)]
struct Overlapped {
    internal: usize,
    internal_high: usize,
    offset: u32,
    offset_high: u32,
    event: *mut core::ffi::c_void,
}

#[cfg(windows)]
unsafe extern "system" {
    fn LockFileEx(
        file: *mut core::ffi::c_void,
        flags: u32,
        reserved: u32,
        low: u32,
        high: u32,
        overlapped: *mut Overlapped,
    ) -> i32;
    fn UnlockFileEx(
        file: *mut core::ffi::c_void,
        reserved: u32,
        low: u32,
        high: u32,
        overlapped: *mut Overlapped,
    ) -> i32;
}

pub struct LockGuard {
    file: File,
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        unlock(&self.file);
    }
}

pub fn lock_world(shard: &World) -> Result<LockGuard, Reason> {
    lock_pack(&shard.root)
}

pub(crate) fn lock_pack(root: &Path) -> Result<LockGuard, Reason> {
    let path = root.join("state/.tick.lock");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|_| Reason::CommitConflict)?;
    if !try_lock_exclusive(&file) {
        return Err(Reason::CommitConflict);
    }
    Ok(LockGuard { file })
}

pub(crate) fn lock_session(root: &Path) -> Result<LockGuard, Reason> {
    let path = root.join("state/.session.lock");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|_| Reason::CommitConflict)?;
    if !lock_exclusive(&file) {
        return Err(Reason::CommitConflict);
    }
    Ok(LockGuard { file })
}

#[cfg(unix)]
fn try_lock_exclusive(file: &File) -> bool {
    unsafe { flock(file.as_raw_fd(), LOCK_EX | LOCK_NB) == 0 }
}

#[cfg(unix)]
fn lock_exclusive(file: &File) -> bool {
    unsafe { flock(file.as_raw_fd(), LOCK_EX) == 0 }
}

#[cfg(unix)]
fn unlock(file: &File) {
    unsafe {
        flock(file.as_raw_fd(), LOCK_UN);
    }
}

#[cfg(windows)]
fn try_lock_exclusive(file: &File) -> bool {
    let mut overlapped = blank_overlapped();
    unsafe {
        LockFileEx(
            file.as_raw_handle(),
            LOCKFILE_FAIL_IMMEDIATELY | LOCKFILE_EXCLUSIVE_LOCK,
            0,
            1,
            0,
            &mut overlapped,
        ) != 0
    }
}

#[cfg(windows)]
fn lock_exclusive(file: &File) -> bool {
    let mut overlapped = blank_overlapped();
    unsafe {
        LockFileEx(
            file.as_raw_handle(),
            LOCKFILE_EXCLUSIVE_LOCK,
            0,
            1,
            0,
            &mut overlapped,
        ) != 0
    }
}

#[cfg(windows)]
fn unlock(file: &File) {
    let mut overlapped = blank_overlapped();
    unsafe {
        UnlockFileEx(file.as_raw_handle(), 0, 1, 0, &mut overlapped);
    }
}

#[cfg(windows)]
fn blank_overlapped() -> Overlapped {
    Overlapped {
        internal: 0,
        internal_high: 0,
        offset: 0,
        offset_high: 0,
        event: core::ptr::null_mut(),
    }
}
