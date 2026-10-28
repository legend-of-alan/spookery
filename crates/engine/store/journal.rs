//! Append-only journal bundles. Each record carries `prev` and `chain`: `chain`
//! is the Blake2s of the previous record's chain and this record's own bytes.
//! A bundle is one file per checkpoint interval. The live bundle is plain
//! JSONL. A closed bundle is gzip and is never appended to again.
//!
//! `journal/base.yaml` is the genesis snapshot, written the first time a
//! checkpoint replaces `save.yaml`. Before that, `save.yaml` is still genesis.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use serde::de::DeserializeOwned;

use crate::error::Reason;
use crate::hash::blake2s_hex;

pub(crate) use super::records::{
    AddedMeter, Arrival, CheckpointRecord, MigrateRecord, Rejected, Step,
};
use super::records::{CheckpointKind, MigrateKind};
pub use super::records::{JournalEntry, RoundRecord};

pub fn hash_now(bytes: &[u8]) -> String {
    format!("blake2s:{}", blake2s_hex(bytes))
}

pub(crate) fn base_rel() -> &'static str {
    "journal/base.yaml"
}

/// The snapshot a checkpoint at `state_rev` publishes.
pub(crate) fn checkpoint_path(pack: &Path, state_rev: u64) -> PathBuf {
    pack.join("state/checkpoints")
        .join(format!("{state_rev}.yaml"))
}

pub(crate) fn head_rel() -> &'static str {
    "journal/head.json"
}

/// The latest checkpoint, written beside the bundles so startup can skip them.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct BundleHead {
    /// Missing on heads written before rules rev 8, which are refused.
    #[serde(default)]
    pub rules_rev: u64,
    pub state_rev: u64,
    pub round_index: u64,
    pub state_hash: String,
    pub chain: String,
}

pub(crate) fn read_head(pack: &Path) -> Result<Option<BundleHead>, JournalFault> {
    let path = pack.join("state").join(head_rel());
    if !path.is_file() {
        return Ok(None);
    }
    let text = fs::read_to_string(&path)
        .map_err(|_| fault(&path, "the checkpoint head could not be read"))?;
    let value: serde_json::Value =
        serde_json::from_str(&text).map_err(|err| fault(&path, &err.to_string()))?;
    let rules_rev = value.get("rules_rev").and_then(|rev| rev.as_u64());
    if let Err(reason) = super::require_rules_rev(rules_rev.unwrap_or(0)) {
        return Err(fault_coded(
            &path,
            reason,
            "RULES_REV: the checkpoint head was written by older rules; start a fresh save",
        ));
    }
    Ok(Some(read_json("head.json", &text)?))
}

pub(crate) fn migrate_entry(
    pack_rev_before: &str,
    pack_rev_after: &str,
    state_rev_before: u64,
    state_rev_after: u64,
    now_hash: &str,
    meters: &[AddedMeter],
) -> JournalEntry {
    JournalEntry::Migrate(MigrateRecord {
        kind: MigrateKind::Migrate,
        pack_rev_before: pack_rev_before.to_string(),
        pack_rev_after: pack_rev_after.to_string(),
        state_rev_before,
        state_rev_after,
        now_hash: now_hash.to_string(),
        meters: meters.to_vec(),
        prev: String::new(),
        chain: String::new(),
    })
}

#[derive(Debug)]
pub struct JournalFault {
    pub file: String,
    pub detail: String,
    pub reason: Reason,
}

/// Parse every complete record. A torn last line is left unread. The chain is not checked.
pub fn read_entries(pack: &Path) -> Result<Vec<JournalEntry>, JournalFault> {
    let dir = pack.join("state/journal");
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    refuse_old_layout(&dir)?;
    let mut entries = Vec::new();
    for (name, path) in bundle_files(&dir)? {
        entries.extend(parse_complete(&name, &read_bundle(&name, &path)?)?);
    }
    Ok(entries)
}

/// Read the bundles and check that each record's chain matches the one before it.
pub fn read_verified(pack: &Path) -> Result<Vec<JournalEntry>, JournalFault> {
    let entries = read_entries(pack)?;
    check_chain(pack, &entries)?;
    Ok(entries)
}

pub(crate) fn check_chain(pack: &Path, entries: &[JournalEntry]) -> Result<(), JournalFault> {
    if entries.is_empty() {
        return Ok(());
    }
    let mut prev = genesis_hash(pack)?;
    for entry in entries {
        if entry.prev() != prev {
            return Err(fault_name(
                &bundle_name_of(pack, entry),
                "the record does not follow the previous hash",
            ));
        }
        let expect = chain_of(entry);
        if entry.chain() != expect {
            return Err(fault_name(
                &bundle_name_of(pack, entry),
                "the chain hash does not cover this record",
            ));
        }
        prev = expect;
    }
    Ok(())
}

/// Blake2s of `prev` and the record bytes with `chain` omitted.
pub(crate) fn chain_hash(prev: &str, body: &str) -> String {
    let mut bytes = Vec::with_capacity(prev.len() + 1 + body.len());
    bytes.extend(prev.as_bytes());
    bytes.push(0);
    bytes.extend(body.as_bytes());
    hash_now(&bytes)
}

pub(crate) fn checkpoint_entry(round_index: u64, state_rev: u64, state_hash: &str) -> JournalEntry {
    JournalEntry::Checkpoint(CheckpointRecord {
        kind: CheckpointKind::Checkpoint,
        round_index,
        state_rev,
        state_hash: state_hash.to_string(),
        prev: String::new(),
        chain: String::new(),
    })
}

/// Chain field of one sealed line.
pub(crate) fn sealed_chain(line: &str) -> Result<String, JournalFault> {
    let entry = parse_line("bundle", line.trim_end())?;
    Ok(entry.chain().to_string())
}

/// `prev` is the chain to continue. The returned line ends with a newline.
pub fn seal(entry: &JournalEntry, prev: &str) -> String {
    let mut sealed = entry.clone();
    sealed.set_prev(prev.to_string());
    sealed.set_chain(String::new());
    let body = emit_entry(&sealed);
    let chain = chain_hash(prev, body.trim_end());
    sealed.set_chain(chain);
    emit_entry(&sealed)
}

/// Chain of the last live record, or of the checkpoint head when the live bundle is empty.
pub(crate) fn read_chain_tip(pack: &Path) -> Result<String, JournalFault> {
    if let Some(path) = find_live_bundle(pack) {
        let name = path
            .file_name()
            .map(|file| file.to_string_lossy().into_owned())
            .unwrap_or_default();
        let text = read_bundle(&name, &path)?;
        if let Some(line) = find_last_line(&text) {
            return check_line_chain(&name, line);
        }
    }
    if let Some(head) = read_head(pack)? {
        return Ok(head.chain);
    }
    genesis_hash(pack)
}

/// Records in the plain live bundle. Closed bundles stay unread.
pub(crate) fn read_live(pack: &Path) -> Result<Vec<JournalEntry>, JournalFault> {
    let dir = pack.join("state/journal");
    if dir.is_dir() {
        refuse_old_layout(&dir)?;
    }
    let Some(path) = find_live_bundle(pack) else {
        return Ok(Vec::new());
    };
    let name = path
        .file_name()
        .map(|file| file.to_string_lossy().into_owned())
        .unwrap_or_default();
    parse_complete(&name, &read_bundle(&name, &path)?)
}

pub fn genesis_hash(pack: &Path) -> Result<String, JournalFault> {
    let base = pack.join("state").join(base_rel());
    let path = if base.is_file() {
        base
    } else {
        pack.join("state/save.yaml")
    };
    let bytes = fs::read(&path).map_err(|_| fault(&path, "the snapshot could not be read"))?;
    Ok(hash_now(&bytes))
}

pub fn latest_checkpoint(entries: &[JournalEntry]) -> Option<&CheckpointRecord> {
    entries.iter().rev().find_map(|entry| match entry {
        JournalEntry::Checkpoint(record) => Some(record),
        _ => None,
    })
}

/// Keys of every round record, such as `round-0001`, in order.
#[allow(dead_code)]
pub fn list_round_keys(pack: &Path) -> Vec<String> {
    let Ok(entries) = read_entries(pack) else {
        return Vec::new();
    };
    let mut keys: Vec<String> = entries
        .iter()
        .filter(|entry| matches!(entry, JournalEntry::Round(_)))
        .map(format_record_key)
        .collect();
    keys.sort();
    keys
}

/// The stored line for a record key such as `round-0001` or `migrate-0002`, without a newline.
#[allow(dead_code)]
pub fn read_record_line(pack: &Path, key: &str) -> Result<String, JournalFault> {
    let entries = read_entries(pack)?;
    let found = entries
        .into_iter()
        .find(|entry| format_record_key(entry) == key);
    match found {
        Some(entry) => Ok(emit_entry(&entry).trim_end().to_string()),
        None => Err(fault_name(key, "the journal record is missing")),
    }
}

/// Where a logical record lives, so a test can edit that line.
pub struct Located {
    pub path: PathBuf,
}

pub fn locate(pack: &Path, key: &str) -> Result<Located, JournalFault> {
    let dir = pack.join("state/journal");
    for (file_name, path) in bundle_files(&dir)? {
        let text = read_bundle(&file_name, &path)?;
        for raw in text.lines() {
            if raw.is_empty() {
                continue;
            }
            let entry = parse_line(&file_name, raw)?;
            if format_record_key(&entry) == key {
                return Ok(Located { path });
            }
        }
    }
    Err(fault_name(key, "the journal record is missing"))
}

#[allow(dead_code)]
pub fn bundles(pack: &Path) -> Result<Vec<(String, PathBuf)>, JournalFault> {
    bundle_files(&pack.join("state/journal"))
}

/// Records in each bundle, in file order. A torn tail is not included.
pub(crate) fn bundle_entries(
    pack: &Path,
) -> Result<Vec<(String, Vec<JournalEntry>)>, JournalFault> {
    let dir = pack.join("state/journal");
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    refuse_old_layout(&dir)?;
    let mut out = Vec::new();
    for (name, path) in bundle_files(&dir)? {
        let entries = parse_complete(&name, &read_bundle(&name, &path)?)?;
        out.push((name, entries));
    }
    Ok(out)
}

/// Plain live bundle, if one is open.
pub fn find_live_bundle(pack: &Path) -> Option<PathBuf> {
    let files = bundle_files(&pack.join("state/journal")).ok()?;
    let (name, path) = files.last()?;
    if name.ends_with(".jsonl") {
        Some(path.clone())
    } else {
        None
    }
}

/// Checkpoint record at the end of a plain bundle, if that is the last line.
pub(crate) fn trailing_checkpoint(path: &Path) -> Option<CheckpointRecord> {
    let text = fs::read_to_string(path).ok()?;
    let line = find_last_line(&text)?;
    match parse_line("bundle", line).ok()? {
        JournalEntry::Checkpoint(record) => Some(record),
        _ => None,
    }
}

/// The checkpoint hash is already in `save.yaml`, so the bundle is finished.
/// Until then it stays the live file and startup can still replay it.
pub(crate) fn is_published(pack: &Path, path: &Path) -> bool {
    let Some(mark) = trailing_checkpoint(path) else {
        return false;
    };
    let Ok(bytes) = fs::read(pack.join("state/save.yaml")) else {
        return false;
    };
    hash_now(&bytes) == mark.state_hash
}

/// Next append lands here. The file may not exist yet.
/// A plain bundle that already ends in a published checkpoint is skipped, so a
/// failed gzip cannot take the next round.
pub(crate) fn next_bundle_path(pack: &Path) -> PathBuf {
    let dir = pack.join("state/journal");
    let files = bundle_files(&dir).unwrap_or_default();
    match files.last() {
        Some((name, path)) if name.ends_with(".jsonl") && !is_published(pack, path) => path.clone(),
        Some((name, _)) => {
            let index = bundle_index(name).unwrap_or(0).saturating_add(1);
            dir.join(format!("bundle-{index:04}.jsonl"))
        }
        None => dir.join("bundle-0000.jsonl"),
    }
}

pub fn emit_entry(entry: &JournalEntry) -> String {
    let json = match entry {
        JournalEntry::Round(record) => serde_json::to_string(record).expect("round"),
        JournalEntry::Migrate(record) => serde_json::to_string(record).expect("migrate"),
        JournalEntry::Checkpoint(record) => serde_json::to_string(record).expect("checkpoint"),
    };
    format!("{json}\n")
}

fn chain_of(entry: &JournalEntry) -> String {
    let mut bare = entry.clone();
    bare.set_chain(String::new());
    let body = emit_entry(&bare);
    chain_hash(entry.prev(), body.trim_end())
}

/// Key naming one record inside the bundles: `round-0001`, `migrate-0002` or `checkpoint-0003`.
pub fn format_record_key(entry: &JournalEntry) -> String {
    match entry {
        JournalEntry::Round(record) => format!("round-{:04}", record.round_index),
        JournalEntry::Migrate(record) => format!("migrate-{:04}", record.state_rev_after),
        JournalEntry::Checkpoint(record) => format!("checkpoint-{:04}", record.round_index),
    }
}

fn bundle_name_of(pack: &Path, entry: &JournalEntry) -> String {
    locate(pack, &format_record_key(entry))
        .map(|found| {
            found
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("bundle")
                .to_string()
        })
        .unwrap_or_else(|_| "bundle".to_string())
}

fn parse_line(name: &str, line: &str) -> Result<JournalEntry, JournalFault> {
    let value: serde_json::Value =
        serde_json::from_str(line).map_err(|err| fault_name(name, &err.to_string()))?;
    let kind = value.get("kind").and_then(|kind| kind.as_str());
    if kind == Some("checkpoint") {
        return Ok(JournalEntry::Checkpoint(read_json(name, line)?));
    }
    if kind == Some("migrate") {
        return Ok(JournalEntry::Migrate(read_json(name, line)?));
    }
    let rules_rev = value.get("rules_rev").and_then(|rev| rev.as_u64());
    parse_round(name, line).map(JournalEntry::Round).map_err(
        |fault| match super::require_rules_rev(rules_rev.unwrap_or(0)) {
            Ok(()) => fault,
            Err(reason) => fault_coded(
                Path::new(name),
                reason,
                "RULES_REV: this record was written by older rules; start a fresh save",
            ),
        },
    )
}

fn refuse_old_layout(dir: &Path) -> Result<(), JournalFault> {
    let entries =
        fs::read_dir(dir).map_err(|_| fault(dir, "the log directory could not be read"))?;
    for entry in entries {
        let entry = entry.map_err(|_| fault(dir, "the log directory could not be read"))?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("presence-") && name.ends_with(".json") {
            return Err(fault_coded(
                &entry.path(),
                Reason::RulesRev,
                "RULES_REV: presence records were written by older rules; start a fresh save",
            ));
        }
        if (name.starts_with("tick-") && name.ends_with(".jsonl"))
            || (name.starts_with("migrate-") && name.ends_with(".json"))
        {
            return Err(fault_coded(
                &entry.path(),
                Reason::RulesRev,
                "RULES_REV: one-file journal records were written by older rules; start a fresh save",
            ));
        }
    }
    Ok(())
}

fn bundle_files(dir: &Path) -> Result<Vec<(String, PathBuf)>, JournalFault> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let entries =
        fs::read_dir(dir).map_err(|_| fault(dir, "the log directory could not be read"))?;
    let mut by_index: BTreeMap<u32, (String, PathBuf)> = BTreeMap::new();
    for entry in entries {
        let entry = entry.map_err(|_| fault(dir, "the log directory could not be read"))?;
        let name = entry.file_name();
        let name = name.to_string_lossy().into_owned();
        let Some(index) = bundle_index(&name) else {
            continue;
        };
        let replace = match by_index.get(&index) {
            Some((have, _)) => name.ends_with(".gz") || !have.ends_with(".gz"),
            None => true,
        };
        if replace {
            by_index.insert(index, (name, entry.path()));
        }
    }
    Ok(by_index.into_values().collect())
}

fn bundle_index(name: &str) -> Option<u32> {
    let rest = name.strip_prefix("bundle-")?;
    let digits = rest
        .strip_suffix(".jsonl.gz")
        .or_else(|| rest.strip_suffix(".jsonl"))?;
    if digits.len() != 4 || !digits.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

fn find_last_line(text: &str) -> Option<&str> {
    let complete = trim_torn_tail(text);
    complete.lines().rev().find(|line| !line.is_empty())
}

fn check_line_chain(name: &str, line: &str) -> Result<String, JournalFault> {
    let entry = parse_line(name, line)?;
    let expect = chain_of(&entry);
    if entry.chain() != expect {
        return Err(fault_name(
            name,
            "the chain hash does not cover this record",
        ));
    }
    Ok(expect)
}

fn trim_torn_tail(text: &str) -> &str {
    if text.ends_with('\n') {
        text
    } else {
        match text.rfind('\n') {
            Some(at) => &text[..=at],
            None => "",
        }
    }
}

fn parse_complete(name: &str, text: &str) -> Result<Vec<JournalEntry>, JournalFault> {
    let mut entries = Vec::new();
    for line in trim_torn_tail(text).lines() {
        if line.is_empty() {
            continue;
        }
        entries.push(parse_line(name, line)?);
    }
    Ok(entries)
}

fn read_bundle(name: &str, path: &Path) -> Result<String, JournalFault> {
    if name.ends_with(".gz") {
        let file =
            File::open(path).map_err(|_| fault(path, "the journal bundle could not be read"))?;
        let mut text = String::new();
        GzDecoder::new(file)
            .read_to_string(&mut text)
            .map_err(|_| fault(path, "the journal bundle could not be read"))?;
        return Ok(text);
    }
    fs::read_to_string(path).map_err(|_| fault(path, "the journal bundle could not be read"))
}

/// Gzip `plain` into memory. The commit path writes and flushes the bytes.
pub fn gzip_bytes(plain: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
    enc.write_all(plain)?;
    enc.finish()
}

pub fn gunzip_bytes(bytes: &[u8]) -> Result<Vec<u8>, Reason> {
    let mut out = Vec::new();
    GzDecoder::new(bytes)
        .read_to_end(&mut out)
        .map_err(|_| Reason::Schema)?;
    Ok(out)
}

pub(crate) fn parse_round(name: &str, text: &str) -> Result<RoundRecord, JournalFault> {
    let raw: RoundRecord<crate::effect::EffectRecord> = read_json(name, text)?;
    let effects = raw
        .effects
        .into_iter()
        .map(crate::effect::Effect::try_from)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|reason| fault_coded(Path::new(name), reason, reason.token()))?;
    Ok(RoundRecord {
        round_index: raw.round_index,
        rules_rev: raw.rules_rev,
        state_rev_before: raw.state_rev_before,
        state_rev_after: raw.state_rev_after,
        world_time_before: raw.world_time_before,
        world_time_after: raw.world_time_after,
        presence_humans: raw.presence_humans,
        actors_present: raw.actors_present,
        seed: raw.seed,
        last_round_at: raw.last_round_at,
        intents: raw.intents,
        rejected: raw.rejected,
        effects,
        arrivals: raw.arrivals,
        fast_forward: raw.fast_forward,
        dice: raw.dice,
        steps_run: raw.steps_run,
        memories: raw.memories,
        now_hash: raw.now_hash,
        prev: raw.prev,
        chain: raw.chain,
    })
}

fn read_json<T: DeserializeOwned>(name: &str, text: &str) -> Result<T, JournalFault> {
    let mut de = serde_json::Deserializer::from_str(text.trim_end());
    serde_path_to_error::deserialize(&mut de).map_err(|err| JournalFault {
        file: name.to_string(),
        detail: err.to_string(),
        reason: Reason::Schema,
    })
}

fn fault(path: &Path, detail: &str) -> JournalFault {
    fault_coded(path, Reason::Schema, detail)
}

fn fault_coded(path: &Path, reason: Reason, detail: &str) -> JournalFault {
    JournalFault {
        file: path.to_string_lossy().replace('\\', "/"),
        detail: detail.to_string(),
        reason,
    }
}

fn fault_name(name: &str, detail: &str) -> JournalFault {
    JournalFault {
        file: name.to_string(),
        detail: detail.to_string(),
        reason: Reason::Schema,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrate_records_round_trip() {
        let migrate = emit_entry(&migrate_entry(
            "blake2s:aa",
            "blake2s:bb",
            1,
            2,
            "blake2s:cc",
            &[],
        ));
        let JournalEntry::Migrate(parsed) =
            parse_line("migrate-0002.json", migrate.trim_end()).unwrap()
        else {
            panic!("not a migrate record");
        };
        assert_eq!(parsed.pack_rev_before, "blake2s:aa");
        assert_eq!(parsed.pack_rev_after, "blake2s:bb");
        assert_eq!(parsed.state_rev_before, 1);
        assert_eq!(parsed.state_rev_after, 2);
        assert_eq!(parsed.now_hash, "blake2s:cc");
        assert_eq!(emit_entry(&JournalEntry::Migrate(parsed)), migrate);
    }

    #[test]
    fn a_sealed_record_chains_to_the_one_before_it() {
        let round = include_str!("../fixtures/round.jsonl");
        let parsed = parse_round("round.jsonl", round).unwrap();
        let entry = JournalEntry::Round(parsed);
        let prev = hash_now(b"genesis");
        let line = seal(&entry, &prev);
        let again = parse_line("bundle-0000.jsonl", line.trim_end()).unwrap();
        assert_eq!(again.prev(), prev);
        assert_eq!(again.chain(), chain_of(&again));
        let next = seal(&entry, again.chain());
        let next_entry = parse_line("bundle-0000.jsonl", next.trim_end()).unwrap();
        assert_eq!(next_entry.prev(), again.chain());
        assert_ne!(next_entry.chain(), again.chain());
    }

    #[test]
    fn a_record_and_head_from_older_rules_are_rules_rev() {
        let old = r#"{"tick_index":1,"rules_rev":7,"state_rev_before":0}"#;
        assert_eq!(
            parse_line("bundle-0000.jsonl", old).unwrap_err().reason,
            Reason::RulesRev
        );
        let pack = crate::testing::temp_pack("old-head");
        let head = pack.join("state").join(head_rel());
        fs::create_dir_all(head.parent().unwrap()).unwrap();
        fs::write(
            &head,
            r#"{"state_rev":1,"tick_index":1,"state_hash":"","chain":""}"#,
        )
        .unwrap();
        assert_eq!(read_head(&pack).unwrap_err().reason, Reason::RulesRev);
        let _ = fs::remove_dir_all(&pack);
    }

    #[test]
    fn golden_round_and_migrate_round_trip() {
        let round = include_str!("../fixtures/round.jsonl");
        let parsed = parse_round("round.jsonl", round).unwrap();
        assert_eq!(emit_entry(&JournalEntry::Round(parsed.clone())), round);
        assert_eq!(parsed.arrivals[0].via, "stairs");
        assert_eq!(parsed.arrivals[0].arrived_at, 1082);
        assert!(parsed.effects.iter().any(|effect| {
            matches!(
                effect.class,
                crate::effect::EffectKind::Location {
                    arrived_at: Some(1082),
                    ..
                }
            )
        }));

        let migrate = include_str!("../fixtures/migrate.json");
        let parsed = parse_line("migrate.json", migrate.trim_end()).unwrap();
        assert_eq!(emit_entry(&parsed), migrate);
    }
}
