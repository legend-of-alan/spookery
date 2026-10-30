//! Check the journal against the save and the checkpoint snapshots.

use std::fs;
use std::path::Path;

use crate::journal::{self, JournalEntry};
use crate::store::codec::emit_save;
use crate::Reason;

use crate::round::replay::{
    apply_entry, checkpoint_diverged, load_base, open_for_replay, read_now_bytes, replay_journal,
    state_rev_of, DivergenceKind, ReplayDivergence, ReplayError,
};

#[derive(Debug)]
pub struct VerifyReport {
    pub state_rev: u64,
    pub applied: u64,
    /// The `state_rev` of each migrate that carried a hand edit. Full verify fills it.
    pub trusted_edits: Vec<u64>,
}

/// Read the journal, replay it, and check checkpoints and `state/save.yaml`.
pub fn verify_pack(pack: &Path, from: Option<&Path>) -> Result<VerifyReport, ReplayError> {
    if !pack.join("pack.yaml").is_file() {
        return Err(ReplayError::Reason(Reason::NotInSpec));
    }
    if !pack.join("state/save.yaml").is_file()
        && !pack.join("state").join(journal::base_rel()).is_file()
    {
        return Err(ReplayError::Reason(Reason::NotInSpec));
    }
    let records = journal::read_entries(pack)?;
    if records.is_empty() && from.is_none() {
        let genesis = read_genesis(pack)?;
        verify_save_file(pack, &records, &genesis)?;
        return Ok(VerifyReport {
            state_rev: state_rev_of(&genesis)?,
            applied: 0,
            trusted_edits: Vec::new(),
        });
    }
    let cap = records.iter().map(JournalEntry::state_rev_after).max();
    let report = replay_journal(pack, from, cap).map_err(|err| name_save_line(pack, err))?;
    let trusted_edits = records
        .iter()
        .filter_map(|entry| match entry {
            JournalEntry::Migrate(record) if record.edited => Some(record.state_rev_after),
            _ => None,
        })
        .collect();
    let verified = VerifyReport {
        state_rev: report.state_rev,
        applied: report.applied,
        trusted_edits,
    };
    if from.is_some() {
        return Ok(verified);
    }
    journal::check_chain(pack, &records)?;
    let genesis = read_genesis(pack)?;
    let bundles = journal::bundle_entries(pack)?;
    bundle_starts(pack, &genesis, &bundles)?;
    verify_save_file(pack, &records, &genesis)?;
    Ok(verified)
}

/// Check every chain hash, then replay the latest bundle from the snapshot it starts at.
pub fn verify_quick(pack: &Path) -> Result<VerifyReport, ReplayError> {
    let records = journal::read_entries(pack)?;
    journal::check_chain(pack, &records)?;
    let genesis = read_genesis(pack)?;
    let bundles = journal::bundle_entries(pack)?;
    let Some((start, entries)) = bundle_starts(pack, &genesis, &bundles)?.pop() else {
        verify_save_file(pack, &records, &genesis)?;
        return Ok(VerifyReport {
            state_rev: state_rev_of(&genesis)?,
            applied: 0,
            trusted_edits: Vec::new(),
        });
    };
    let (name, _) = bundles.last().expect("bundle");
    let end = check_bundle_end(pack, name, entries, replay_bundle(pack, &start, entries))?;
    verify_save_file(pack, &records, &genesis)?;
    Ok(VerifyReport {
        state_rev: state_rev_of(&end)?,
        applied: entries.iter().filter(|entry| entry.changes_state()).count() as u64,
        trusted_edits: Vec::new(),
    })
}

type BundleStart<'a> = (Vec<u8>, &'a [JournalEntry]);

fn bundle_starts<'a>(
    pack: &Path,
    genesis: &[u8],
    bundles: &'a [(String, Vec<JournalEntry>)],
) -> Result<Vec<BundleStart<'a>>, ReplayError> {
    let mut starts = Vec::with_capacity(bundles.len());
    let mut current = genesis.to_vec();
    for (index, (name, entries)) in bundles.iter().enumerate() {
        starts.push((current, entries.as_slice()));
        if index + 1 == bundles.len() {
            break;
        }
        let Some(mark) = journal::latest_checkpoint(entries) else {
            return Err(ReplayDivergence::new(
                DivergenceKind::Checkpoint,
                0,
                None,
                format!("{name}: a closed bundle has no checkpoint"),
            )
            .into());
        };
        let diverged = |detail: &str| {
            checkpoint_diverged(
                mark.state_rev,
                mark.round_index,
                format!("{name}: {detail}"),
            )
        };
        current = fs::read(journal::checkpoint_path(pack, mark.state_rev))
            .map_err(|_| diverged("missing snapshot"))?;
        if journal::hash_now(&current) != mark.state_hash {
            return Err(diverged("state hash differed").replayed(current).into());
        }
    }
    Ok(starts)
}

fn check_bundle_end(
    pack: &Path,
    name: &str,
    entries: &[JournalEntry],
    end: Result<Vec<u8>, ReplayError>,
) -> Result<Vec<u8>, ReplayError> {
    let end = end.map_err(|err| name_save_line(pack, err))?;
    if let Some(mark) = journal::latest_checkpoint(entries) {
        if journal::hash_now(&end) != mark.state_hash {
            let detail = format!("{name}: state hash differed");
            return Err(
                checkpoint_diverged(mark.state_rev, mark.round_index, detail)
                    .replayed(end)
                    .into(),
            );
        }
    }
    Ok(end)
}

fn verify_save_file(
    pack: &Path,
    records: &[JournalEntry],
    genesis: &[u8],
) -> Result<(), ReplayError> {
    let live = fs::read(pack.join("state/save.yaml"))
        .map_err(|_| ReplayError::Reason(Reason::NotInSpec))?;
    let now = load_base(&live)?;
    if records.is_empty() {
        let canonical = emit_save(&now)
            .map(|text| text.into_bytes())
            .map_err(ReplayError::Reason)?;
        if live != canonical {
            let diverged = ReplayDivergence::new(
                DivergenceKind::Save,
                now.state_rev,
                None,
                Reason::HandEdit.token(),
            );
            return Err(with_save_line(diverged.replayed(canonical), &live));
        }
    }
    if let Some(mark) = journal::latest_checkpoint(records) {
        if journal::hash_now(&live) != mark.state_hash {
            let snap = fs::read(journal::checkpoint_path(pack, mark.state_rev)).ok();
            let diverged = ReplayDivergence {
                replayed: snap,
                ..ReplayDivergence::new(
                    DivergenceKind::Save,
                    mark.state_rev,
                    Some(mark.round_index),
                    "state hash differed",
                )
            };
            return Err(with_save_line(diverged, &live));
        }
    } else if live != genesis {
        let diverged = ReplayDivergence::new(
            DivergenceKind::Save,
            now.state_rev,
            None,
            "state hash differed",
        );
        return Err(with_save_line(diverged.replayed(genesis.to_vec()), &live));
    }
    Ok(())
}

/// Name the first line of `live` that differs from the replayed bytes.
fn with_save_line(mut diverged: ReplayDivergence, live: &[u8]) -> ReplayError {
    diverged.save_line = diverged.replayed.as_deref().and_then(|replayed| {
        let live_text = std::str::from_utf8(live).ok()?;
        let replayed_text = std::str::from_utf8(replayed).ok()?;
        crate::world::first_diff_line(live_text, replayed_text)
    });
    diverged.into()
}

/// A replay divergence names the first differing line of the save, or of the base without one.
fn name_save_line(pack: &Path, err: ReplayError) -> ReplayError {
    let ReplayError::Diverged(diverged) = err else {
        return err;
    };
    let live = fs::read(pack.join("state/save.yaml"))
        .or_else(|_| fs::read(pack.join("state").join(journal::base_rel())))
        .unwrap_or_default();
    with_save_line(diverged, &live)
}

fn read_genesis(pack: &Path) -> Result<Vec<u8>, ReplayError> {
    let base = pack.join("state").join(journal::base_rel());
    if base.is_file() {
        return fs::read(&base).map_err(|_| ReplayError::Reason(Reason::Schema));
    }
    fs::read(pack.join("state/save.yaml")).map_err(|_| ReplayError::Reason(Reason::NotInSpec))
}

fn replay_bundle(
    pack: &Path,
    start: &[u8],
    entries: &[JournalEntry],
) -> Result<Vec<u8>, ReplayError> {
    let mut world = open_for_replay(pack, start)?;
    let mut changed = false;
    for entry in entries {
        apply_entry(&mut world, entry)?;
        changed |= entry.changes_state();
    }
    if changed {
        read_now_bytes(&world)
    } else {
        Ok(start.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{verify_pack, verify_quick};
    use crate::round::play_pack_once;
    use crate::testing::temp_pack;

    #[test]
    fn verify_names_a_round_whose_hash_no_longer_matches() {
        let pack = temp_pack("replay-tamper");
        play_pack_once(&pack).unwrap();
        let path = pack.join("state/journal/bundle-0000.jsonl");
        let text = fs::read_to_string(&path).unwrap();
        let swapped = text.replacen("\"now_hash\":\"blake2s:", "\"now_hash\":\"blake2s:00", 1);
        assert_ne!(swapped, text);
        fs::write(&path, swapped).unwrap();
        let err = verify_pack(&pack, None).unwrap_err();
        let message = err.to_string();
        assert!(message.contains("state_rev 1"), "{message}");
        assert!(message.contains("round 1"), "{message}");
        assert!(message.contains("now_hash differed"), "{message}");
        let _ = fs::remove_dir_all(&pack);
    }

    #[test]
    fn verify_and_quick_verify_reject_tampered_save_after_checkpoint() {
        let pack = temp_pack("replay-save-tamper");
        let pack_yaml = pack.join("pack.yaml");
        let mut cfg = fs::read_to_string(&pack_yaml).unwrap();
        cfg.push_str("\ncheckpoint_every: 2\n");
        fs::write(&pack_yaml, cfg).unwrap();
        play_pack_once(&pack).unwrap();
        play_pack_once(&pack).unwrap();
        let save = pack.join("state/save.yaml");
        let text = fs::read_to_string(&save).unwrap();
        let tampered = text.replacen("state_rev: 2", "state_rev: 99", 1);
        assert_ne!(tampered, text);
        fs::write(&save, tampered).unwrap();
        let full_err = verify_pack(&pack, None).unwrap_err().to_string();
        assert!(full_err.contains("state_rev 2 save"), "{full_err}");
        assert!(full_err.contains("save.yaml:"), "{full_err}");
        let quick_err = verify_quick(&pack).unwrap_err().to_string();
        assert!(quick_err.contains("state_rev 2 save"), "{quick_err}");
        assert!(quick_err.contains("save.yaml:"), "{quick_err}");
        let _ = fs::remove_dir_all(&pack);
    }
}
