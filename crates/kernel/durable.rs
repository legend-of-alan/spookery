use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

use crate::error::Reason;

pub(crate) fn tmp_path(target: &Path) -> PathBuf {
    let mut name = target.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    target.with_file_name(name)
}

pub(crate) fn fsync_file(path: &Path) -> Result<(), Reason> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|_| Reason::CommitConflict)?;
    file.sync_all().map_err(|_| Reason::CommitConflict)?;
    Ok(())
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
        return Ok(());
    }
    #[cfg(not(windows))]
    {
        let file = std::fs::File::open(path).map_err(|_| Reason::CommitConflict)?;
        file.sync_all().map_err(|_| Reason::CommitConflict)?;
        Ok(())
    }
}
