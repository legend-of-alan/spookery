//! One record per `now.yaml` commit, plus the replay base.
//!
//! A tick record stays `log/tick-NNNN.jsonl`. A presence commit writes
//! `log/presence-NNNN.json`. A migrate commit writes `log/migrate-NNNN.json`.
//! `NNNN` is `state_rev` after the commit for presence and migrate, and the
//! tick index for a tick file, which is the name the rest of the tree already
//! reads. Each record carries `now_hash`, the Blake2s of the `now.yaml` bytes
//! that commit wrote. `log/base.yaml` is replay input, not the live save.

use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::error::Reason;
use crate::hash::blake2s_hex;

pub fn hash_now(bytes: &[u8]) -> String {
    format!("blake2s:{}", blake2s_hex(bytes))
}

pub fn presence_file(state_rev: u64) -> String {
    format!("presence-{state_rev:04}.json")
}

pub fn migrate_file(state_rev: u64) -> String {
    format!("migrate-{state_rev:04}.json")
}

pub fn base_rel() -> &'static str {
    "log/base.yaml"
}

/// Insert `now_hash` before the closing brace of one JSON object.
pub fn stamp_now_hash(jsonl: &str, now_bytes: &[u8]) -> Result<String, Reason> {
    let trimmed = jsonl.trim_end();
    let body = trimmed.strip_suffix('}').ok_or(Reason::Schema)?;
    if body.contains("\"now_hash\"") {
        return Err(Reason::Schema);
    }
    let hash = hash_now(now_bytes);
    Ok(format!("{body},\"now_hash\":\"{hash}\"}}\n"))
}

pub fn emit_presence(
    state_rev_before: u64,
    state_rev_after: u64,
    humans: u32,
    actors_present: &[String],
    now_hash: &str,
) -> String {
    let actors = serde_json::to_string(actors_present).expect("actor ids");
    format!(
        "{{\"kind\":\"presence\",\"state_rev_before\":{state_rev_before},\"state_rev_after\":{state_rev_after},\"humans\":{humans},\"actors_present\":{actors},\"now_hash\":\"{now_hash}\"}}\n"
    )
}

pub fn emit_migrate(
    pack_rev_before: &str,
    pack_rev_after: &str,
    state_rev_before: u64,
    state_rev_after: u64,
    now_hash: &str,
) -> String {
    format!(
        "{{\"kind\":\"migrate\",\"pack_rev_before\":{before},\"pack_rev_after\":{after},\"state_rev_before\":{state_rev_before},\"state_rev_after\":{state_rev_after},\"now_hash\":{hash}}}\n",
        before = serde_json::to_string(pack_rev_before).expect("pack_rev"),
        after = serde_json::to_string(pack_rev_after).expect("pack_rev"),
        hash = serde_json::to_string(now_hash).expect("now_hash"),
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresenceRecord {
    pub state_rev_before: u64,
    pub state_rev_after: u64,
    pub humans: u32,
    pub actors_present: Vec<String>,
    pub now_hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MigrateRecord {
    pub pack_rev_before: String,
    pub pack_rev_after: String,
    pub state_rev_before: u64,
    pub state_rev_after: u64,
    pub now_hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TickRecord {
    pub state_rev_before: u64,
    pub state_rev_after: u64,
    pub tick_index: u64,
    pub now_hash: String,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JournalEntry {
    Tick(TickRecord),
    Presence(PresenceRecord),
    Migrate(MigrateRecord),
}

impl JournalEntry {
    pub fn state_rev_after(&self) -> u64 {
        match self {
            Self::Tick(record) => record.state_rev_after,
            Self::Presence(record) => record.state_rev_after,
            Self::Migrate(record) => record.state_rev_after,
        }
    }

    pub fn state_rev_before(&self) -> u64 {
        match self {
            Self::Tick(record) => record.state_rev_before,
            Self::Presence(record) => record.state_rev_before,
            Self::Migrate(record) => record.state_rev_before,
        }
    }

    pub fn now_hash(&self) -> &str {
        match self {
            Self::Tick(record) => &record.now_hash,
            Self::Presence(record) => &record.now_hash,
            Self::Migrate(record) => &record.now_hash,
        }
    }

    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::Tick(_) => "tick",
            Self::Presence(_) => "presence",
            Self::Migrate(_) => "migrate",
        }
    }

    pub fn tick_index(&self) -> Option<u64> {
        match self {
            Self::Tick(record) => Some(record.tick_index),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub struct JournalFault {
    pub file: String,
    pub detail: String,
}

pub fn load_journal(pack: &Path) -> Result<Vec<JournalEntry>, JournalFault> {
    let dir = pack.join("state/log");
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut names = fs::read_dir(&dir)
        .map_err(|_| fault(&dir, "the log directory could not be read"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| fault(&dir, "the log directory could not be read"))?;
    names.sort_by_key(|entry| entry.file_name());
    let mut entries = Vec::new();
    for entry in names {
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();
        let path = entry.path();
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(_) => {
                return Err(fault(&path, "the journal record could not be read"));
            }
        };
        if name.starts_with("tick-") && name.ends_with(".jsonl") {
            entries.push(JournalEntry::Tick(parse_tick(&name, &text)?));
        } else if name.starts_with("presence-") && name.ends_with(".json") {
            entries.push(JournalEntry::Presence(parse_presence(&name, &text)?));
        } else if name.starts_with("migrate-") && name.ends_with(".json") {
            entries.push(JournalEntry::Migrate(parse_migrate(&name, &text)?));
        }
    }
    entries.sort_by(|left, right| {
        left.state_rev_after()
            .cmp(&right.state_rev_after())
            .then_with(|| left.kind_name().cmp(right.kind_name()))
    });
    Ok(entries)
}

fn parse_tick(name: &str, text: &str) -> Result<TickRecord, JournalFault> {
    let object = object(name, text)?;
    Ok(TickRecord {
        state_rev_before: req_u64(name, &object, "state_rev_before")?,
        state_rev_after: req_u64(name, &object, "state_rev_after")?,
        tick_index: req_u64(name, &object, "tick_index")?,
        now_hash: req_str(name, &object, "now_hash")?,
        text: text.to_string(),
    })
}

pub fn parse_presence(name: &str, text: &str) -> Result<PresenceRecord, JournalFault> {
    let object = object(name, text)?;
    if object.get("kind").and_then(Value::as_str) != Some("presence") {
        return Err(fault_name(name, "kind is not presence"));
    }
    let actors = object
        .get("actors_present")
        .and_then(Value::as_array)
        .ok_or_else(|| fault_name(name, "actors_present is missing"))?;
    let mut actors_present = Vec::new();
    for actor in actors {
        actors_present.push(
            actor
                .as_str()
                .ok_or_else(|| fault_name(name, "actors_present holds a non-string"))?
                .to_string(),
        );
    }
    Ok(PresenceRecord {
        state_rev_before: req_u64(name, &object, "state_rev_before")?,
        state_rev_after: req_u64(name, &object, "state_rev_after")?,
        humans: req_u64(name, &object, "humans")?
            .try_into()
            .map_err(|_| fault_name(name, "humans does not fit"))?,
        actors_present,
        now_hash: req_str(name, &object, "now_hash")?,
    })
}

pub fn parse_migrate(name: &str, text: &str) -> Result<MigrateRecord, JournalFault> {
    let object = object(name, text)?;
    if object.get("kind").and_then(Value::as_str) != Some("migrate") {
        return Err(fault_name(name, "kind is not migrate"));
    }
    Ok(MigrateRecord {
        pack_rev_before: req_str(name, &object, "pack_rev_before")?,
        pack_rev_after: req_str(name, &object, "pack_rev_after")?,
        state_rev_before: req_u64(name, &object, "state_rev_before")?,
        state_rev_after: req_u64(name, &object, "state_rev_after")?,
        now_hash: req_str(name, &object, "now_hash")?,
    })
}

fn object(name: &str, text: &str) -> Result<serde_json::Map<String, Value>, JournalFault> {
    let value: Value = serde_json::from_str(text.trim_end())
        .map_err(|_| fault_name(name, "the record is not JSON"))?;
    value
        .as_object()
        .cloned()
        .ok_or_else(|| fault_name(name, "the record is not an object"))
}

fn req_u64(
    name: &str,
    object: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<u64, JournalFault> {
    object
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| fault_name(name, &format!("{key} is missing")))
}

fn req_str(
    name: &str,
    object: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<String, JournalFault> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| fault_name(name, &format!("{key} is missing")))
}

fn fault(path: &Path, detail: &str) -> JournalFault {
    JournalFault {
        file: path.to_string_lossy().replace('\\', "/"),
        detail: detail.to_string(),
    }
}

fn fault_name(name: &str, detail: &str) -> JournalFault {
    JournalFault {
        file: name.to_string(),
        detail: detail.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presence_and_migrate_records_round_trip() {
        let presence = emit_presence(0, 1, 1, &["pc_rowan".to_string()], "blake2s:aa");
        let parsed = parse_presence("presence-0001.json", &presence).unwrap();
        assert_eq!(parsed.state_rev_before, 0);
        assert_eq!(parsed.state_rev_after, 1);
        assert_eq!(parsed.humans, 1);
        assert_eq!(parsed.actors_present, vec!["pc_rowan".to_string()]);
        assert_eq!(parsed.now_hash, "blake2s:aa");
        assert_eq!(
            emit_presence(
                parsed.state_rev_before,
                parsed.state_rev_after,
                parsed.humans,
                &parsed.actors_present,
                &parsed.now_hash
            ),
            presence
        );

        let migrate = emit_migrate("blake2s:aa", "blake2s:bb", 1, 2, "blake2s:cc");
        let parsed = parse_migrate("migrate-0002.json", &migrate).unwrap();
        assert_eq!(parsed.pack_rev_before, "blake2s:aa");
        assert_eq!(parsed.pack_rev_after, "blake2s:bb");
        assert_eq!(parsed.state_rev_before, 1);
        assert_eq!(parsed.state_rev_after, 2);
        assert_eq!(parsed.now_hash, "blake2s:cc");
        assert_eq!(
            emit_migrate(
                &parsed.pack_rev_before,
                &parsed.pack_rev_after,
                parsed.state_rev_before,
                parsed.state_rev_after,
                &parsed.now_hash
            ),
            migrate
        );
    }

    #[test]
    fn now_hash_stamps_the_tick_object() {
        let stamped = stamp_now_hash("{\"tick_index\":1}\n", b"now").unwrap();
        assert!(stamped.starts_with("{\"tick_index\":1,\"now_hash\":\"blake2s:"));
        assert!(stamped.ends_with("\"}\n"));
        let hash = hash_now(b"now");
        assert!(stamped.contains(&hash));
        assert!(stamp_now_hash(&stamped, b"now").is_err());
    }
}
