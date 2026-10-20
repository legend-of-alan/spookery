//! Replay the journal and report a hidden fact in a player view.
//!
//! The hidden set is the canon's hidden facts minus that PC's `known_facts`.
//! It is a second reading of the pack, not the firewall predicate.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use spookery_kernel::api::canon::{load_canon, Canon};
use spookery_kernel::api::diagnostic::reason_of;
use spookery_kernel::api::effect::EffectKind;
use spookery_kernel::api::journal::{self, JournalEntry, TickRecord};
use spookery_kernel::api::model::Now;
use spookery_kernel::api::perceive::perceive_at;
use spookery_kernel::api::store::codec::load_save;
use spookery_kernel::api::store::talk_log::{self, TapeRecord};
use spookery_kernel::api::Reason;
use spookery_round::{replay_states, ReplayError};

use crate::host::frames;

pub struct AuditReport {
    pub text: String,
    pub failed: bool,
}

pub enum AuditError {
    Reason(Reason),
    Replay(String),
}

impl std::fmt::Display for AuditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Reason(reason) => write!(f, "{reason}"),
            Self::Replay(detail) => write!(f, "{detail}"),
        }
    }
}

struct Snap {
    state_rev: u64,
    tick_index: u64,
    now: Now,
}

struct Finding {
    state_rev: u64,
    pc: String,
    line: String,
}

struct Note {
    seq: u64,
    speaker: String,
    recipient: String,
}

struct Secret {
    id: String,
    text: String,
}

/// Read the pack, replay every committed state, and render the report.
/// Writes nothing, takes no lock, and binds no port.
pub fn audit_pack(pack: &Path, strict: bool) -> Result<AuditReport, AuditError> {
    if !pack.join("pack.yaml").is_file() || !pack.join("state/save.yaml").is_file() {
        return Err(AuditError::Reason(Reason::NotInSpec));
    }
    let canon = load_canon(pack).map_err(|diags| AuditError::Reason(reason_of(diags)))?;
    let (snaps, ticks) = load_snaps(pack)?;
    let tape = tape_records(pack)?;
    report_of(&canon, &snaps, &ticks, &tape, strict)
}

fn tape_records(pack: &Path) -> Result<Vec<TapeRecord>, AuditError> {
    let path = pack.join("state/journal/session.jsonl");
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let bytes = fs::read(&path).map_err(|_| AuditError::Reason(Reason::Schema))?;
    talk_log::parse_records(&bytes).map_err(AuditError::Reason)
}

fn load_snaps(pack: &Path) -> Result<(Vec<Snap>, Vec<TickRecord>), AuditError> {
    let records = journal::load_journal(pack)
        .map_err(|fault| AuditError::Replay(format!("{}: {}", fault.file, fault.detail)))?;
    let ticks = records
        .iter()
        .filter_map(|entry| match entry {
            JournalEntry::Tick(record) => Some(record.clone()),
            _ => None,
        })
        .collect();
    let mut snaps = Vec::new();
    if records.is_empty() {
        let bytes = fs::read(pack.join("state/save.yaml"))
            .map_err(|_| AuditError::Reason(Reason::NotInSpec))?;
        snaps.push(snap_of(&bytes)?);
        return Ok((snaps, ticks));
    }
    let base = pack.join("state").join(journal::base_rel());
    let genesis = if base.is_file() {
        fs::read(&base)
    } else {
        fs::read(pack.join("state/save.yaml"))
    }
    .map_err(|_| AuditError::Reason(Reason::Schema))?;
    snaps.push(snap_of(&genesis)?);
    for step in replay_states(pack).map_err(replay_error)? {
        snaps.push(snap_of(&step.save_yaml)?);
    }
    Ok((snaps, ticks))
}

fn snap_of(bytes: &[u8]) -> Result<Snap, AuditError> {
    let text = std::str::from_utf8(bytes).map_err(|_| AuditError::Reason(Reason::Schema))?;
    let now = load_save(text).map_err(|diags| AuditError::Reason(reason_of(diags)))?;
    Ok(Snap {
        state_rev: now.state_rev,
        tick_index: now.tick_index,
        now,
    })
}

fn replay_error(err: ReplayError) -> AuditError {
    match err {
        ReplayError::Reason(reason) => AuditError::Reason(reason),
        ReplayError::Diverged(diverged) => AuditError::Replay(diverged.to_string()),
    }
}

fn report_of(
    canon: &Canon,
    snaps: &[Snap],
    ticks: &[TickRecord],
    tape: &[TapeRecord],
    strict: bool,
) -> Result<AuditReport, AuditError> {
    let pcs = pcs_of(snaps);
    let mut failed_pcs = BTreeSet::new();
    let mut findings = Vec::new();
    for snap in snaps {
        for pc in &pcs {
            let Some(actor) = snap.now.actors.get(pc) else {
                continue;
            };
            if !actor.is_pc() {
                continue;
            }
            let view = perceive_at(canon, &snap.now, &format!("player:{pc}"))
                .map_err(AuditError::Reason)?;
            let line = frames::player_perceive_line(&view);
            let line = line.trim_end_matches(['\n', '\r']).to_string();
            if line_leaks(&line, &unknown_hidden(canon, actor.known_facts())) {
                failed_pcs.insert(pc.clone());
                findings.push(Finding {
                    state_rev: snap.state_rev,
                    pc: pc.clone(),
                    line,
                });
            }
        }
    }
    let narrated = narrate_findings(canon, snaps, ticks);
    for finding in &narrated {
        failed_pcs.insert(finding.pc.clone());
    }
    findings.extend(narrated);
    findings.sort_by(|left, right| {
        left.state_rev
            .cmp(&right.state_rev)
            .then(left.pc.cmp(&right.pc))
            .then(left.line.cmp(&right.line))
    });

    let (notes, used_final) = notes_of(canon, snaps, tape);
    let failed = !findings.is_empty() || (strict && !notes.is_empty());
    let how = if used_final {
        "notes use the final state"
    } else {
        "notes follow the journal state at the tape line's tick"
    };
    let mut text = String::new();
    for pc in &pcs {
        if failed_pcs.contains(pc) {
            text.push_str(&format!("FAIL {pc}\n"));
        } else {
            text.push_str(&format!("pass {pc}\n"));
        }
    }
    for finding in &findings {
        text.push_str(&format!(
            "state_rev {} {} {}\n",
            finding.state_rev, finding.pc, finding.line
        ));
    }
    for tick in ticks {
        for line in &tick.dice {
            text.push_str(&format!("dice {} {line}\n", tick.tick_index));
        }
    }
    for note in &notes {
        text.push_str(&format!(
            "note seq {} speaker {} recipient {}\n",
            note.seq, note.speaker, note.recipient
        ));
    }
    text.push_str(how);
    text.push('\n');
    Ok(AuditReport { text, failed })
}

fn pcs_of(snaps: &[Snap]) -> Vec<String> {
    let mut pcs = BTreeSet::new();
    for snap in snaps {
        for (id, actor) in &snap.now.actors {
            if actor.is_pc() {
                pcs.insert(id.clone());
            }
        }
    }
    pcs.into_iter().collect()
}

fn unknown_hidden(canon: &Canon, known: &[String]) -> Vec<Secret> {
    let mut secrets = Vec::new();
    for (id, fact) in &canon.facts {
        if !fact.hidden || known.iter().any(|item| item == id) {
            continue;
        }
        secrets.push(Secret {
            id: id.clone(),
            text: fact.text.clone(),
        });
    }
    secrets.sort_by(|left, right| left.id.cmp(&right.id));
    secrets
}

fn line_leaks(line: &str, secrets: &[Secret]) -> bool {
    secrets.iter().any(|secret| {
        contains_quoted_id(line, &secret.id)
            || (!secret.text.is_empty() && contains_rendered(line, &secret.text))
    })
}

/// A fact id matches only as a JSON string, so `fact_ledger` does not hit `fact_ledger_short`.
fn contains_quoted_id(line: &str, id: &str) -> bool {
    if id.is_empty() {
        return false;
    }
    let quoted = serde_json::to_string(id).expect("string");
    line.contains(&quoted)
}

fn contains_rendered(line: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    if line.contains(needle) {
        return true;
    }
    let encoded = serde_json::to_string(needle).expect("string");
    let fragment = &encoded[1..encoded.len() - 1];
    fragment != needle && line.contains(fragment)
}

fn narrate_findings(canon: &Canon, snaps: &[Snap], ticks: &[TickRecord]) -> Vec<Finding> {
    let mut findings = Vec::new();
    for tick in ticks {
        let Some(snap) = snaps
            .iter()
            .find(|snap| snap.state_rev == tick.state_rev_after)
        else {
            continue;
        };
        for effect in &tick.effects {
            let EffectKind::Narrate { to, fact_ids, text } = &effect.class else {
                continue;
            };
            for pc in to {
                let Some(actor) = snap.now.actors.get(pc) else {
                    continue;
                };
                if !actor.is_pc() {
                    continue;
                }
                let secrets = unknown_hidden(canon, actor.known_facts());
                let leaked = secrets.iter().any(|secret| {
                    fact_ids.iter().any(|id| id == &secret.id)
                        || (!secret.text.is_empty() && text.contains(&secret.text))
                });
                if !leaked {
                    continue;
                }
                findings.push(Finding {
                    state_rev: tick.state_rev_after,
                    pc: pc.clone(),
                    line: text.clone(),
                });
            }
        }
    }
    findings
}

fn notes_of(canon: &Canon, snaps: &[Snap], tape: &[TapeRecord]) -> (Vec<Note>, bool) {
    let mut notes = Vec::new();
    let mut seen = BTreeSet::new();
    let mut used_final = false;
    let Some(final_snap) = snaps.last() else {
        return (notes, true);
    };
    for line in tape {
        let matched = snaps.iter().find(|snap| snap.tick_index == line.tick_index);
        let snap = match matched {
            Some(snap) => snap,
            None => {
                used_final = true;
                final_snap
            }
        };
        for recipient in &line.recipients {
            let known: &[String] = match snap.now.actors.get(recipient) {
                Some(actor) => actor.known_facts(),
                None => &[],
            };
            let leaked = unknown_hidden(canon, known)
                .iter()
                .any(|secret| !secret.text.is_empty() && line.text.contains(&secret.text));
            if !leaked {
                continue;
            }
            let key = (line.seq, line.from.clone(), recipient.clone());
            if !seen.insert(key) {
                continue;
            }
            notes.push(Note {
                seq: line.seq,
                speaker: line.from.clone(),
                recipient: recipient.clone(),
            });
        }
    }
    notes.sort_by(|left, right| {
        left.seq
            .cmp(&right.seq)
            .then(left.speaker.cmp(&right.speaker))
            .then(left.recipient.cmp(&right.recipient))
    });
    (notes, used_final)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::{audit_pack, contains_quoted_id};

    #[test]
    fn a_shorter_fact_id_does_not_match_a_longer_one() {
        let line = r#"{"id":"fact_ledger_short"}"#;
        assert!(!contains_quoted_id(line, "fact_ledger"));
        assert!(contains_quoted_id(line, "fact_ledger_short"));
    }

    #[test]
    fn audit_leaves_the_pack_unchanged() {
        let root = temp_docks();
        let tape = root.join("state/journal/session.jsonl");
        fs::create_dir_all(tape.parent().unwrap()).unwrap();
        fs::write(
            &tape,
            "{\"seq\":1,\"from\":\"gm\",\"address\":\"actor\",\"place\":null,\"actor\":\"pc_rowan\",\"recipients\":[\"pc_rowan\"],\"text\":\"line0\",\"tick_index\":0,\"world_time\":1080}\n",
        )
        .unwrap();
        let before = dir_bytes(&root.join("state"));
        audit_pack(&root, false).unwrap_or_else(|err| panic!("audit: {err}"));
        assert_eq!(dir_bytes(&root.join("state")), before);
        assert!(!root.join("state/.session.lock").exists());
        let _ = fs::remove_dir_all(&root);
    }

    fn temp_docks() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "spook-audit-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&root);
        copy_dir(
            &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packs/oneshot-docks"),
            &root,
        );
        root
    }

    fn copy_dir(from: &Path, to: &Path) {
        fs::create_dir_all(to).unwrap();
        for entry in fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let dest = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_dir(&entry.path(), &dest);
            } else {
                fs::copy(entry.path(), dest).unwrap();
            }
        }
    }

    fn dir_bytes(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        let mut out = Vec::new();
        walk(root, &mut out);
        out.sort();
        out
    }

    fn walk(dir: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if entry.file_type().unwrap().is_dir() {
                walk(&path, out);
            } else {
                let bytes = fs::read(&path).unwrap();
                out.push((path, bytes));
            }
        }
    }
}
