//! The session tape at `state/tape.jsonl`.

use std::fs::{self, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::store::durability::{fsync_dir, store_io, write_atomic};
use crate::Reason;
use spookery_protocol::Speaker;

/// Size bound for `state/tape.jsonl`. An append that would pass it first drops the oldest half.
const MAX_BYTES: u64 = 1024 * 1024;

const WINDOW_LINES: usize = 64;
const SCROLLBACK_LINES: usize = 256;

/// One tape line, written by the engine and relayed by the host table to players and the GM.
/// Field order is the order of the bytes on disk.
/// `from` is stored as the `Speaker`'s text. `place` and `actor` may be null. A missing key
/// is not a record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TapeRecord {
    pub seq: u64,
    pub from: Speaker,
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
pub fn emit_tape_record(record: &TapeRecord) -> String {
    serde_json::to_string(record).expect("tape json")
}

/// Append one tape record. The pack lock makes the host the only writer.
/// `seq` is set one past the last complete line. Drops a trailing partial line and does not open
/// a `Draft`. When the line would pass `MAX_BYTES`, the oldest whole lines go first, keeping the
/// newest half.
pub fn append_record(root: &Path, mut record: TapeRecord) -> Result<TapeRecord, Reason> {
    let state = root.join("state");
    fs::create_dir_all(&state).map_err(store_io)?;
    let path = state.join("tape.jsonl");
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(store_io)?;
    let len = file.metadata().map_err(store_io)?.len();
    if len > MAX_BYTES {
        return Err(Reason::OutOfRange);
    }
    let bytes = fs::read(&path).map_err(store_io)?;
    let mut lines = bytes.rsplit(|byte| *byte == b'\n');
    let torn = lines.next().unwrap_or_default();
    let end = (bytes.len() - torn.len()) as u64;
    let last_seq = match lines.next() {
        Some(last) => {
            let last = last.strip_suffix(b"\r").unwrap_or(last);
            parse_record(std::str::from_utf8(last).map_err(|_| Reason::Schema)?)?.seq
        }
        None => 0,
    };
    record.seq = last_seq.checked_add(1).ok_or(Reason::OutOfRange)?;
    let mut line = emit_tape_record(&record);
    if line.is_empty() || line.bytes().any(|byte| byte == b'\n' || byte == b'\r') {
        return Err(Reason::Schema);
    }
    line.push('\n');
    if end + line.len() as u64 > MAX_BYTES {
        let from = end.saturating_sub(MAX_BYTES / 2) as usize;
        let start = bytes[from..end as usize]
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(end as usize, |index| from + index + 1);
        let mut kept = bytes[start..end as usize].to_vec();
        kept.extend_from_slice(line.as_bytes());
        write_atomic(&path, &kept)?;
        return Ok(record);
    }
    if end != len {
        file.set_len(end).map_err(store_io)?;
    }
    file.seek(SeekFrom::End(0)).map_err(store_io)?;
    file.write_all(line.as_bytes()).map_err(store_io)?;
    file.sync_all().map_err(store_io)?;
    fsync_dir(&state)?;
    Ok(record)
}

/// Records with `seq` greater than `since`, at most `WINDOW_LINES` of them.
/// A missing file is empty. A trailing partial line is not a record. A complete
/// line that does not parse is `SCHEMA` for the whole read. A file past 1 MiB is `OUT_OF_RANGE`.
pub fn read_after(root: &Path, since: u64) -> Result<Vec<TapeRecord>, Reason> {
    let mut out = Vec::new();
    for record in read_records(root)? {
        if record.seq > since && out.len() < WINDOW_LINES {
            out.push(record);
        }
    }
    Ok(out)
}

/// The newest `SCROLLBACK_LINES` records whose recipients include `player`, oldest first.
/// The filter runs before the limit.
pub fn read_scrollback(root: &Path, player: &str) -> Result<Vec<TapeRecord>, Reason> {
    let mut matched = Vec::new();
    for record in read_records(root)? {
        if record.recipients.iter().any(|id| id == player) {
            matched.push(record);
        }
    }
    let skip = matched.len().saturating_sub(SCROLLBACK_LINES);
    Ok(matched.split_off(skip))
}

/// Every complete record, oldest first. A missing file is empty. A trailing partial line is
/// not a record. A complete line that does not parse is `SCHEMA`. Past 1 MiB is `OUT_OF_RANGE`.
pub fn read_records(root: &Path) -> Result<Vec<TapeRecord>, Reason> {
    let path = root.join("state/tape.jsonl");
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let len = fs::metadata(&path).map_err(store_io)?.len();
    if len > MAX_BYTES {
        return Err(Reason::OutOfRange);
    }
    let bytes = fs::read(&path).map_err(store_io)?;
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
pub fn last_seq(root: &Path) -> Result<u64, Reason> {
    Ok(read_records(root)?
        .last()
        .map(|record| record.seq)
        .unwrap_or(0))
}

fn parse_record(line: &str) -> Result<TapeRecord, Reason> {
    serde_json::from_str(line).map_err(|_| Reason::Schema)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::fresh_dir;

    #[test]
    fn an_appended_say_keeps_the_tape_bytes() {
        let root = fresh_dir("tape-bytes");
        let stored = append_record(&root, sample(0, "pc_rowan", "line0")).unwrap();
        let bytes = fs::read(root.join("state/tape.jsonl")).unwrap();
        let line = "{\"seq\":1,\"from\":\"gm\",\"address\":\"actor\",\"place\":null,\"actor\":\"pc_rowan\",\"recipients\":[\"pc_rowan\"],\"text\":\"line0\",\"round_index\":0,\"world_time\":1080}\n";
        assert_eq!(bytes, line.as_bytes());
        assert_eq!(parse_record(line.trim_end()).unwrap(), stored);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn read_scrollback_keeps_the_newest_lines_for_that_player() {
        let root = fresh_dir("tape-tail");
        let mut lines = String::new();
        for index in 0..300 {
            lines.push_str(&emit_tape_record(&sample(
                index as u64 + 1,
                "pc_wren",
                &format!("w{index}"),
            )));
            lines.push('\n');
        }
        for index in 0..3 {
            lines.push_str(&emit_tape_record(&sample(
                301 + index,
                "pc_rowan",
                &format!("line{index}"),
            )));
            lines.push('\n');
        }
        write_tape(&root, &lines);
        let tail = read_scrollback(&root, "pc_rowan").unwrap();
        let texts: Vec<_> = tail.iter().map(|record| record.text.as_str()).collect();
        assert_eq!(texts, ["line0", "line1", "line2"]);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn read_scrollback_keeps_the_256_line_cap() {
        let root = fresh_dir("tape-cap");
        let mut lines = String::new();
        for index in 0..300 {
            lines.push_str(&emit_tape_record(&sample(
                index as u64 + 1,
                "pc_rowan",
                &format!("line{index}"),
            )));
            lines.push('\n');
        }
        write_tape(&root, &lines);
        let tail = read_scrollback(&root, "pc_rowan").unwrap();
        assert_eq!(tail.len(), 256);
        assert_eq!(tail[0].text, "line44");
        assert_eq!(tail[255].text, "line299");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_tape_past_one_mib_is_out_of_range() {
        let root = fresh_dir("tape-cap-bytes");
        let path = root.join("state/tape.jsonl");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, vec![b'\n'; (MAX_BYTES as usize) + 1]).unwrap();
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
            from: Speaker::Gm,
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
        let path = root.join("state/tape.jsonl");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, lines).unwrap();
    }
}
