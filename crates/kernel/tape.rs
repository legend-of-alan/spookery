use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use crate::durable::fsync_dir;
use crate::error::Reason;
use crate::lock::lock_session;

/// Hard cap for `state/log/session.jsonl`.
pub const MAX_TAPE_BYTES: u64 = 1024 * 1024;

const DEFAULT_TAPE_LIMIT: usize = 64;
const MAX_TAPE_LIMIT: usize = 256;

/// Append one session tape line under `state/.session.lock`.
/// Assigns `seq`, drops a trailing partial line, and does not open a `Draft`.
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
    let len = file.metadata().map_err(|_| Reason::CommitConflict)?.len();
    if len > MAX_TAPE_BYTES {
        return Err(Reason::OutOfRange);
    }
    let (end, last_seq) = tail_seq(&mut file, len)?;
    let seq = last_seq.checked_add(1).ok_or(Reason::CommitConflict)?;
    let mut line = body(seq);
    if line.is_empty() || line.bytes().any(|byte| byte == b'\n' || byte == b'\r') {
        return Err(Reason::Schema);
    }
    line.push('\n');
    let next_len = end + line.len() as u64;
    if next_len > MAX_TAPE_BYTES {
        return Err(Reason::OutOfRange);
    }
    if end != len {
        file.set_len(end).map_err(|_| Reason::CommitConflict)?;
    }
    file.seek(SeekFrom::End(0))
        .map_err(|_| Reason::CommitConflict)?;
    file.write_all(line.as_bytes())
        .map_err(|_| Reason::CommitConflict)?;
    file.sync_all().map_err(|_| Reason::CommitConflict)?;
    fsync_dir(&log_dir)?;
    Ok(seq)
}

/// Tape lines with `seq` greater than `after_seq`, at most `limit` of them.
/// `limit` defaults to 64 and is clamped to 256. A missing file is empty.
/// A trailing partial line is not a record. A complete line that does not
/// parse is `SCHEMA` for the whole read. A file past 1 MiB is `OUT_OF_RANGE`.
pub fn read_session(
    root: &Path,
    after_seq: u64,
    limit: Option<usize>,
) -> Result<Vec<String>, Reason> {
    let limit = match limit {
        Some(limit) => limit.min(MAX_TAPE_LIMIT),
        None => DEFAULT_TAPE_LIMIT,
    };
    let mut out = Vec::new();
    for line in load_kept(root)? {
        let seq = session_seq(line.as_bytes()).ok_or(Reason::Schema)?;
        if seq > after_seq && out.len() < limit {
            out.push(line);
        }
    }
    Ok(out)
}

/// Last complete `seq`, or 0 when the tape has no record.
pub fn session_end_seq(root: &Path) -> Result<u64, Reason> {
    match load_kept(root)?.last() {
        Some(line) => session_seq(line.as_bytes()).ok_or(Reason::Schema),
        None => Ok(0),
    }
}

fn load_kept(root: &Path) -> Result<Vec<String>, Reason> {
    let path = root.join("state/log/session.jsonl");
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let _lock = lock_session(root)?;
    let len = fs::metadata(&path)
        .map_err(|_| Reason::CommitConflict)?
        .len();
    if len > MAX_TAPE_BYTES {
        return Err(Reason::OutOfRange);
    }
    let bytes = fs::read(&path).map_err(|_| Reason::CommitConflict)?;
    let Some(last_nl) = bytes.iter().rposition(|byte| *byte == b'\n') else {
        return Ok(Vec::new());
    };
    let text = std::str::from_utf8(&bytes[..=last_nl]).map_err(|_| Reason::Schema)?;
    let mut lines = Vec::new();
    for line in text.split_inclusive('\n') {
        let line = line.strip_suffix('\n').unwrap_or(line);
        let line = line.strip_suffix('\r').unwrap_or(line);
        if session_seq(line.as_bytes()).is_none() {
            return Err(Reason::Schema);
        }
        lines.push(line.to_string());
    }
    Ok(lines)
}

/// `(kept_end, last_seq)` from the file tail. A torn suffix is not a record.
/// A complete line that does not parse is `SCHEMA`.
fn tail_seq(file: &mut File, len: u64) -> Result<(u64, u64), Reason> {
    if len == 0 {
        return Ok((0, 0));
    }
    let mut start = len;
    let mut suffix = Vec::new();
    loop {
        let take = start.min(8192);
        if take == 0 {
            break;
        }
        start -= take;
        file.seek(SeekFrom::Start(start))
            .map_err(|_| Reason::CommitConflict)?;
        let mut chunk = vec![0u8; take as usize];
        file.read_exact(&mut chunk)
            .map_err(|_| Reason::CommitConflict)?;
        chunk.append(&mut suffix);
        suffix = chunk;
        if last_line_in_suffix(&suffix, start == 0) {
            break;
        }
    }
    let Some(last_nl) = suffix.iter().rposition(|byte| *byte == b'\n') else {
        return Ok((0, 0));
    };
    let kept_end = start + last_nl as u64 + 1;
    let line_start = suffix[..last_nl]
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    let mut line = &suffix[line_start..last_nl];
    if line.last() == Some(&b'\r') {
        line = &line[..line.len() - 1];
    }
    let seq = session_seq(line).ok_or(Reason::Schema)?;
    Ok((kept_end, seq))
}

fn last_line_in_suffix(suffix: &[u8], at_start: bool) -> bool {
    let Some(last_nl) = suffix.iter().rposition(|byte| *byte == b'\n') else {
        return at_start;
    };
    at_start || suffix[..last_nl].contains(&b'\n')
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
