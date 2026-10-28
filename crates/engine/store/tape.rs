//! The session tape at `state/journal/session.jsonl`.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::Reason;
use crate::lock::lock_session;
use crate::store::durable::fsync_dir;

/// Hard cap for `state/journal/session.jsonl`.
pub const MAX_TAPE_BYTES: u64 = 1024 * 1024;

const DEFAULT_TAPE_LIMIT: usize = 64;
const MAX_TAPE_LIMIT: usize = 256;

/// One tape line. Field order is the order of the bytes on disk.
/// `place` and `actor` may be null. A missing key is not a record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TapeRecord {
    pub seq: u64,
    pub from: String,
    pub address: String,
    #[serde(deserialize_with = "Option::deserialize")]
    pub place: Option<String>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub actor: Option<String>,
    pub recipients: Vec<String>,
    pub text: String,
    pub round_index: u64,
    pub world_time: u64,
}

/// The JSON object written as one tape line, without the trailing newline.
pub fn record_line(record: &TapeRecord) -> String {
    serde_json::to_string(record).expect("tape json")
}

/// Append one tape record under `state/.session.lock`.
/// `body` receives the assigned `seq`. Drops a trailing partial line and does not open a `Draft`.
pub fn append_record(
    root: &Path,
    body: impl FnOnce(u64) -> TapeRecord,
) -> Result<TapeRecord, Reason> {
    let _lock = lock_session(root)?;
    let log_dir = root.join("state/journal");
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
    let record = body(seq);
    let mut line = record_line(&record);
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
    Ok(record)
}

/// Records with `seq` greater than `after_seq`, at most `limit` of them.
/// `limit` defaults to 64 and is clamped to 256. A missing file is empty.
/// A trailing partial line is not a record. A complete line that does not
/// parse is `SCHEMA` for the whole read. A file past 1 MiB is `OUT_OF_RANGE`.
pub fn read_session(
    root: &Path,
    after_seq: u64,
    limit: Option<usize>,
) -> Result<Vec<TapeRecord>, Reason> {
    let limit = match limit {
        Some(limit) => limit.min(MAX_TAPE_LIMIT),
        None => DEFAULT_TAPE_LIMIT,
    };
    let mut out = Vec::new();
    for record in read_records(root)? {
        if record.seq > after_seq && out.len() < limit {
            out.push(record);
        }
    }
    Ok(out)
}

/// The newest `n` records whose recipients include `player`, oldest first.
/// `n` is clamped to 256. The filter runs before the limit.
pub fn read_tail(root: &Path, player: &str, n: usize) -> Result<Vec<TapeRecord>, Reason> {
    let n = n.min(MAX_TAPE_LIMIT);
    let mut matched = Vec::new();
    for record in read_records(root)? {
        if record.recipients.iter().any(|id| id == player) {
            matched.push(record);
        }
    }
    let skip = matched.len().saturating_sub(n);
    Ok(matched.split_off(skip))
}

/// Every complete record, oldest first, under `state/.session.lock`. A missing file is empty.
pub fn read_records(root: &Path) -> Result<Vec<TapeRecord>, Reason> {
    if !root.join("state/journal/session.jsonl").is_file() {
        return Ok(Vec::new());
    }
    let _lock = lock_session(root)?;
    read_records_unlocked(root)
}

/// `read_records` without the lock, so it writes nothing. A trailing partial line is not
/// a record. A complete line that does not parse is `SCHEMA`. Past 1 MiB is `OUT_OF_RANGE`.
pub fn read_records_unlocked(root: &Path) -> Result<Vec<TapeRecord>, Reason> {
    let path = root.join("state/journal/session.jsonl");
    if !path.is_file() {
        return Ok(Vec::new());
    }
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
    let mut records = Vec::new();
    for line in text.split_inclusive('\n') {
        let line = line.strip_suffix('\n').unwrap_or(line);
        let line = line.strip_suffix('\r').unwrap_or(line);
        records.push(parse_record(line)?);
    }
    Ok(records)
}

/// Last complete `seq`, or 0 when the tape has no record.
pub fn session_end_seq(root: &Path) -> Result<u64, Reason> {
    Ok(read_records(root)?
        .last()
        .map(|record| record.seq)
        .unwrap_or(0))
}

fn parse_record(line: &str) -> Result<TapeRecord, Reason> {
    serde_json::from_str(line).map_err(|_| Reason::Schema)
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
    let seq = parse_record(std::str::from_utf8(line).map_err(|_| Reason::Schema)?)?.seq;
    Ok((kept_end, seq))
}

fn last_line_in_suffix(suffix: &[u8], at_start: bool) -> bool {
    let Some(last_nl) = suffix.iter().rposition(|byte| *byte == b'\n') else {
        return at_start;
    };
    at_start || suffix[..last_nl].contains(&b'\n')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_appended_say_keeps_the_tape_bytes() {
        let root = temp_root("bytes");
        let stored = append_record(&root, |seq| TapeRecord {
            seq,
            from: "gm".to_string(),
            address: "actor".to_string(),
            place: None,
            actor: Some("pc_rowan".to_string()),
            recipients: vec!["pc_rowan".to_string()],
            text: "line0".to_string(),
            round_index: 0,
            world_time: 1080,
        })
        .unwrap();
        let bytes = fs::read(root.join("state/journal/session.jsonl")).unwrap();
        let line = "{\"seq\":1,\"from\":\"gm\",\"address\":\"actor\",\"place\":null,\"actor\":\"pc_rowan\",\"recipients\":[\"pc_rowan\"],\"text\":\"line0\",\"round_index\":0,\"world_time\":1080}\n";
        assert_eq!(bytes, line.as_bytes());
        assert_eq!(parse_record(line.trim_end()).unwrap(), stored);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn read_tail_keeps_the_newest_lines_for_that_player() {
        let root = temp_root("tail");
        let mut lines = String::new();
        for index in 0..300 {
            lines.push_str(&record_line(&sample(
                index as u64 + 1,
                "pc_wren",
                &format!("w{index}"),
            )));
            lines.push('\n');
        }
        for index in 0..3 {
            lines.push_str(&record_line(&sample(
                301 + index,
                "pc_rowan",
                &format!("line{index}"),
            )));
            lines.push('\n');
        }
        write_tape(&root, &lines);
        let tail = read_tail(&root, "pc_rowan", 1000).unwrap();
        let texts: Vec<_> = tail.iter().map(|record| record.text.as_str()).collect();
        assert_eq!(texts, ["line0", "line1", "line2"]);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn read_tail_and_read_session_keep_the_256_line_cap() {
        let root = temp_root("cap");
        let mut lines = String::new();
        for index in 0..300 {
            lines.push_str(&record_line(&sample(
                index as u64 + 1,
                "pc_rowan",
                &format!("line{index}"),
            )));
            lines.push('\n');
        }
        write_tape(&root, &lines);
        let tail = read_tail(&root, "pc_rowan", 1000).unwrap();
        assert_eq!(tail.len(), 256);
        assert_eq!(tail[0].text, "line44");
        assert_eq!(tail[255].text, "line299");
        let window = read_session(&root, 0, Some(1000)).unwrap();
        assert_eq!(window.len(), 256);
        assert_eq!(window[0].text, "line0");
        assert_eq!(window[255].text, "line255");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_tape_past_one_mib_is_out_of_range() {
        let root = temp_root("cap-bytes");
        let path = root.join("state/journal/session.jsonl");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, vec![b'\n'; (MAX_TAPE_BYTES as usize) + 1]).unwrap();
        assert_eq!(read_records(&root).unwrap_err(), Reason::OutOfRange);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_line_missing_place_or_actor_is_schema() {
        let missing_place = r#"{"seq":1,"from":"gm","address":"table","actor":null,"recipients":[],"text":"hi","round_index":0,"world_time":0}"#;
        let missing_actor = r#"{"seq":1,"from":"gm","address":"table","place":null,"recipients":[],"text":"hi","round_index":0,"world_time":0}"#;
        assert_eq!(parse_record(missing_place).unwrap_err(), Reason::Schema);
        assert_eq!(parse_record(missing_actor).unwrap_err(), Reason::Schema);
        let present = r#"{"seq":1,"from":"gm","address":"table","place":null,"actor":null,"recipients":[],"text":"hi","round_index":0,"world_time":0}"#;
        assert!(parse_record(present).is_ok());
    }

    fn sample(seq: u64, player: &str, text: &str) -> TapeRecord {
        TapeRecord {
            seq,
            from: "gm".to_string(),
            address: "actor".to_string(),
            place: None,
            actor: Some(player.to_string()),
            recipients: vec![player.to_string()],
            text: text.to_string(),
            round_index: 0,
            world_time: 1080,
        }
    }

    fn write_tape(root: &Path, lines: &str) {
        let path = root.join("state/journal/session.jsonl");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, lines).unwrap();
    }

    fn temp_root(label: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "spook-tape-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("state")).unwrap();
        root
    }
}
