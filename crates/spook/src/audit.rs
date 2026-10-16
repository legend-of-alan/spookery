//! Replay the journal and report a hidden fact in a player view.
//!
//! The hidden set is the canon's hidden facts minus that PC's `known_facts`.
//! It is a second reading of the pack, not the firewall predicate.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use spookery_kernel::canon::{load_canon, Canon};
use spookery_kernel::diagnostic::reason_of;
use spookery_kernel::effect::EffectKind;
use spookery_kernel::journal::{self, JournalEntry, TickRecord};
use spookery_kernel::model::Now;
use spookery_kernel::perceive::perceive_at;
use spookery_kernel::store::codec::load_save;
use spookery_kernel::store::talk_log::MAX_TAPE_BYTES;
use spookery_kernel::Reason;
use spookery_round::{replay_states, ReplayError};

use crate::host::frames::{self, TapeLine};

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
    let tape = read_tape(pack)?;
    report_of(&canon, &snaps, &ticks, &tape, strict)
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
    if !base.is_file() {
        return Err(AuditError::Replay("missing base.yaml".to_string()));
    }
    snaps.push(snap_of(
        &fs::read(&base).map_err(|_| AuditError::Reason(Reason::Schema))?,
    )?);
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

fn read_tape(pack: &Path) -> Result<Vec<TapeLine>, AuditError> {
    let path = pack.join("state/journal/session.jsonl");
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let bytes = fs::read(&path).map_err(|_| AuditError::Reason(Reason::Schema))?;
    if bytes.len() as u64 > MAX_TAPE_BYTES {
        return Err(AuditError::Reason(Reason::OutOfRange));
    }
    let Some(last_nl) = bytes.iter().rposition(|byte| *byte == b'\n') else {
        return Ok(Vec::new());
    };
    let text =
        std::str::from_utf8(&bytes[..=last_nl]).map_err(|_| AuditError::Reason(Reason::Schema))?;
    let mut lines = Vec::new();
    for line in text.split_inclusive('\n') {
        let line = line.trim_end_matches(['\n', '\r']);
        if line.is_empty() {
            continue;
        }
        lines.push(frames::parse_tape(line).map_err(AuditError::Reason)?);
    }
    Ok(lines)
}

fn report_of(
    canon: &Canon,
    snaps: &[Snap],
    ticks: &[TickRecord],
    tape: &[TapeLine],
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

fn notes_of(canon: &Canon, snaps: &[Snap], tape: &[TapeLine]) -> (Vec<Note>, bool) {
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
    use super::contains_quoted_id;

    #[test]
    fn a_shorter_fact_id_does_not_match_a_longer_one() {
        let line = r#"{"id":"fact_ledger_short"}"#;
        assert!(!contains_quoted_id(line, "fact_ledger"));
        assert!(contains_quoted_id(line, "fact_ledger_short"));
    }
}
