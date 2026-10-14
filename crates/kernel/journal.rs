//! One record per `save.yaml` commit, plus the replay base.
//!
//! A tick record stays `journal/tick-NNNN.jsonl`. A presence commit writes
//! `journal/presence-NNNN.json`. A migrate commit writes `journal/migrate-NNNN.json`.
//! `NNNN` is `state_rev` after the commit for presence and migrate, and the
//! tick index for a tick file, which is the name the rest of the tree already
//! reads. Each record carries `now_hash`, the Blake2s of the `save.yaml` bytes
//! that commit wrote. `journal/base.yaml` is replay input, not the live save.

use std::fs;
use std::path::Path;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::effect::Effect;
use crate::error::Reason;
use crate::hash::blake2s_hex;
use crate::intent::Intent;

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
    "journal/base.yaml"
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
    emit_entry(&JournalEntry::Presence(PresenceRecord {
        kind: PresenceKind::Presence,
        state_rev_before,
        state_rev_after,
        humans,
        actors_present: actors_present.to_vec(),
        now_hash: now_hash.to_string(),
    }))
}

pub fn emit_migrate(
    pack_rev_before: &str,
    pack_rev_after: &str,
    state_rev_before: u64,
    state_rev_after: u64,
    now_hash: &str,
) -> String {
    emit_entry(&JournalEntry::Migrate(MigrateRecord {
        kind: MigrateKind::Migrate,
        pack_rev_before: pack_rev_before.to_string(),
        pack_rev_after: pack_rev_after.to_string(),
        state_rev_before,
        state_rev_after,
        now_hash: now_hash.to_string(),
    }))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PresenceKind {
    Presence,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum MigrateKind {
    Migrate,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PresenceRecord {
    kind: PresenceKind,
    pub state_rev_before: u64,
    pub state_rev_after: u64,
    pub humans: u32,
    pub actors_present: Vec<String>,
    pub now_hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MigrateRecord {
    kind: MigrateKind,
    pub pack_rev_before: String,
    pub pack_rev_after: String,
    pub state_rev_before: u64,
    pub state_rev_after: u64,
    pub now_hash: String,
}

/// One refusal stored on a tick line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rejected {
    pub id: String,
    pub reason: String,
}

/// One timed arrival stored on a tick line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Arrival {
    pub actor_id: String,
    pub arrived_at: u64,
    pub to: String,
    pub via: String,
}

/// One tick line. `now_hash` is stamped after the save bytes exist, so a line
/// that has not been stamped omits it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TickRecord {
    pub tick_index: u64,
    #[serde(default)]
    pub rules_rev: u64,
    pub state_rev_before: u64,
    pub state_rev_after: u64,
    pub world_time_before: u64,
    pub world_time_after: u64,
    pub presence_humans: u32,
    pub seed: String,
    pub last_tick_at: String,
    pub intents: Vec<Intent>,
    pub rejected: Vec<Rejected>,
    pub effects: Vec<Effect>,
    pub arrivals: Vec<Arrival>,
    pub fast_forward: bool,
    pub tool_trace: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub now_hash: String,
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
    let dir = pack.join("state/journal");
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

pub fn emit_entry(entry: &JournalEntry) -> String {
    let json = match entry {
        JournalEntry::Tick(record) => serde_json::to_string(record).expect("tick"),
        JournalEntry::Presence(record) => serde_json::to_string(record).expect("presence"),
        JournalEntry::Migrate(record) => serde_json::to_string(record).expect("migrate"),
    };
    format!("{json}\n")
}

pub(crate) fn parse_tick(name: &str, text: &str) -> Result<TickRecord, JournalFault> {
    read_json(name, text)
}

pub fn parse_presence(name: &str, text: &str) -> Result<PresenceRecord, JournalFault> {
    read_json(name, text)
}

pub fn parse_migrate(name: &str, text: &str) -> Result<MigrateRecord, JournalFault> {
    read_json(name, text)
}

fn read_json<T: DeserializeOwned>(name: &str, text: &str) -> Result<T, JournalFault> {
    let mut de = serde_json::Deserializer::from_str(text.trim_end());
    match serde_path_to_error::deserialize(&mut de) {
        Ok(value) => Ok(value),
        Err(err) => Err(fault_name(name, &fault_detail(&err.to_string()))),
    }
}

fn fault_detail(message: &str) -> String {
    if message.contains("ILLEGAL_CLASS") {
        message.to_string()
    } else if message.contains("unknown variant") && message.contains("`clock`") {
        format!("ILLEGAL_CLASS: {message}")
    } else {
        message.to_string()
    }
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

    #[test]
    fn golden_tick_presence_and_migrate_round_trip() {
        let tick = include_str!("fixtures/tick.jsonl");
        let parsed = parse_tick("tick.jsonl", tick).unwrap();
        assert_eq!(emit_entry(&JournalEntry::Tick(parsed.clone())), tick);
        assert_eq!(parsed.arrivals[0].via, "stairs");
        assert_eq!(parsed.arrivals[0].arrived_at, 1082);
        assert!(parsed.effects.iter().any(|effect| {
            matches!(
                effect.class,
                crate::effect::EffectClass::Location {
                    arrived_at: Some(1082),
                    ..
                }
            )
        }));

        let presence = include_str!("fixtures/presence.json");
        let parsed = parse_presence("presence.json", presence).unwrap();
        assert_eq!(emit_entry(&JournalEntry::Presence(parsed)), presence);

        let migrate = include_str!("fixtures/migrate.json");
        let parsed = parse_migrate("migrate.json", migrate).unwrap();
        assert_eq!(emit_entry(&JournalEntry::Migrate(parsed)), migrate);
    }
}
