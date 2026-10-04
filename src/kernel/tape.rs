use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use crate::commit::fsync_dir;
use crate::error::Reason;
use crate::lock::lock_session;

/// Append one session tape line under `state/.session.lock`.
/// Assigns `seq`, drops a trailing partial line, and does not open a `Txn`.
pub fn append_session(root: &Path, body: impl FnOnce(u64) -> String) -> Result<u64, Reason> {
    let _lock = lock_session(root)?;
    let log_dir = root.join("state/log");
    fs::create_dir_all(&log_dir).map_err(|_| Reason::CommitConflict)?;
    let path = log_dir.join("session.jsonl");
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|_| Reason::CommitConflict)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|_| Reason::CommitConflict)?;
    let end = session_kept(&bytes);
    let seq = last_session_seq(&bytes[..end])
        .checked_add(1)
        .ok_or(Reason::CommitConflict)?;
    let mut line = body(seq);
    if line.is_empty() || line.bytes().any(|byte| byte == b'\n' || byte == b'\r') {
        return Err(Reason::Schema);
    }
    line.push('\n');
    if end != bytes.len() {
        file.set_len(end as u64)
            .map_err(|_| Reason::CommitConflict)?;
    }
    file.seek(SeekFrom::End(0))
        .map_err(|_| Reason::CommitConflict)?;
    file.write_all(line.as_bytes())
        .map_err(|_| Reason::CommitConflict)?;
    file.sync_all().map_err(|_| Reason::CommitConflict)?;
    fsync_dir(&log_dir)?;
    Ok(seq)
}

/// Complete tape lines. A missing file is empty. A trailing partial line is not a record.
pub fn read_session(root: &Path) -> Result<Vec<String>, Reason> {
    let path = root.join("state/log/session.jsonl");
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let _lock = lock_session(root)?;
    let bytes = fs::read(&path).map_err(|_| Reason::CommitConflict)?;
    let end = session_kept(&bytes);
    let text = std::str::from_utf8(&bytes[..end]).map_err(|_| Reason::Schema)?;
    let mut lines = Vec::new();
    for line in text.lines() {
        if session_seq(line.as_bytes()).is_some() {
            lines.push(line.to_string());
        }
    }
    Ok(lines)
}

fn session_kept(bytes: &[u8]) -> usize {
    let Some(last_lf) = bytes.iter().rposition(|byte| *byte == b'\n') else {
        return 0;
    };
    let mut end = last_lf + 1;
    while end > 0 {
        let start = bytes[..end - 1]
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map(|index| index + 1)
            .unwrap_or(0);
        let mut line = &bytes[start..end - 1];
        if line.last() == Some(&b'\r') {
            line = &line[..line.len() - 1];
        }
        if session_seq(line).is_some() {
            return end;
        }
        end = start;
    }
    0
}

fn last_session_seq(bytes: &[u8]) -> u64 {
    let end = session_kept(bytes);
    if end == 0 {
        return 0;
    }
    let start = bytes[..end - 1]
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    let mut line = &bytes[start..end - 1];
    if line.last() == Some(&b'\r') {
        line = &line[..line.len() - 1];
    }
    session_seq(line).unwrap_or(0)
}

fn session_seq(line: &[u8]) -> Option<u64> {
    let rest = line.strip_prefix(br#"{"seq":"#)?;
    if rest.first().is_none_or(|byte| !byte.is_ascii_digit()) {
        return None;
    }
    let mut value: u64 = 0;
    for byte in rest {
        if !byte.is_ascii_digit() {
            return Some(value);
        }
        value = value.checked_mul(10)?.checked_add(u64::from(byte - b'0'))?;
    }
    Some(value)
}
