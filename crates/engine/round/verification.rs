//! Check the journal against the save and the checkpoint snapshots.

use std::fs;
use std::path::Path;

use crate::hash::blake2s_tagged;
use crate::store::journal;
use crate::store::records::Record;
use crate::store::save;
use crate::Reason;

use crate::replay::{
    self, checkpoint_diverged, emit_save_bytes, load_save_bytes, state_rev_of, Divergence,
    DivergenceKind, Error,
};

#[derive(Debug)]
pub struct Report {
    pub state_rev: u64,
    pub applied: u64,
    /// The `state_rev` of each migrate that carried a hand edit. Full verify fills it.
    pub trusted_edits: Vec<u64>,
}

/// Read the journal, replay it, and check checkpoints and `state/save.yaml`. A pack with
/// neither a save nor a base is refused `NOT_IN_SPEC`, with or without `from`.
pub fn run(pack: &Path, from: Option<&Path>) -> Result<Report, Error> {
    if !pack.join("pack.yaml").is_file() {
        return Err(Error::Reason(Reason::NotInSpec));
    }
    if !journal::save_path(pack).exists() && !journal::base_path(pack).exists() {
        return Err(Error::Reason(Reason::NotInSpec));
    }
    let bundles = journal::read_bundle_records(pack)?;
    let records = journal::flatten_bundles(&bundles);
    if records.is_empty() && from.is_none() {
        let base = read_base(pack)?;
        check_save(pack, &records, &base)?;
        return Ok(Report {
            state_rev: state_rev_of(&base)?,
            applied: 0,
            trusted_edits: Vec::new(),
        });
    }
    let cap = records.iter().map(Record::state_rev_after).max();
    let report = replay::apply_journal(pack, &records, from, cap)
        .map_err(|err| read_first_diverged_line(pack, err))?;
    let trusted_edits = records
        .iter()
        .filter_map(|entry| match entry {
            Record::Migrate(record) if record.edited => Some(record.state_rev_after),
            _ => None,
        })
        .collect();
    let verified = Report {
        state_rev: report.state_rev,
        applied: report.applied,
        trusted_edits,
    };
    if from.is_some() {
        return Ok(verified);
    }
    journal::check_chain(pack, &bundles)?;
    let base = read_base(pack)?;
    check_bundle_starts(pack, &base, &bundles)?;
    check_save(pack, &records, &base)?;
    Ok(verified)
}

/// Check every chain hash, then replay the latest bundle from the snapshot it starts at.
pub fn run_quick(pack: &Path) -> Result<Report, Error> {
    let bundles = journal::read_bundle_records(pack)?;
    journal::check_chain(pack, &bundles)?;
    let records = journal::flatten_bundles(&bundles);
    let base = read_base(pack)?;
    let Some((start, entries)) = check_bundle_starts(pack, &base, &bundles)?.pop() else {
        check_save(pack, &records, &base)?;
        return Ok(Report {
            state_rev: state_rev_of(&base)?,
            applied: 0,
            trusted_edits: Vec::new(),
        });
    };
    let (bundle, _) = bundles.last().expect("bundle");
    let end = check_bundle_end(pack, bundle, entries, replay(pack, &start, entries))?;
    check_save(pack, &records, &base)?;
    Ok(Report {
        state_rev: state_rev_of(&end)?,
        applied: entries.iter().filter(|entry| entry.changes_state()).count() as u64,
        trusted_edits: Vec::new(),
    })
}

type BundleStart<'a> = (Vec<u8>, &'a [Record]);

fn check_bundle_starts<'a>(
    pack: &Path,
    base: &[u8],
    bundles: &'a [(String, Vec<Record>)],
) -> Result<Vec<BundleStart<'a>>, Error> {
    let mut starts = Vec::with_capacity(bundles.len());
    let mut current = base.to_vec();
    for (index, (bundle, entries)) in bundles.iter().enumerate() {
        starts.push((current, entries.as_slice()));
        if index + 1 == bundles.len() {
            break;
        }
        let Some(checkpoint) = journal::latest_checkpoint(entries) else {
            return Err(Divergence::new(
                DivergenceKind::Checkpoint,
                0,
                None,
                format!("{bundle}: a closed bundle has no checkpoint"),
            )
            .into());
        };
        let diverged = |detail: &str| {
            checkpoint_diverged(
                checkpoint.state_rev,
                checkpoint.round_index,
                format!("{bundle}: {detail}"),
            )
        };
        current = fs::read(journal::checkpoint_path(pack, checkpoint.state_rev))
            .map_err(|_| diverged("missing snapshot"))?;
        if blake2s_tagged(&current) != checkpoint.state_hash {
            return Err(diverged("state hash differed")
                .with_replayed_bytes(current)
                .into());
        }
    }
    Ok(starts)
}

fn check_bundle_end(
    pack: &Path,
    bundle: &str,
    entries: &[Record],
    end: Result<Vec<u8>, Error>,
) -> Result<Vec<u8>, Error> {
    let end = end.map_err(|err| read_first_diverged_line(pack, err))?;
    if let Some(checkpoint) = journal::latest_checkpoint(entries) {
        if blake2s_tagged(&end) != checkpoint.state_hash {
            let detail = format!("{bundle}: state hash differed");
            return Err(
                checkpoint_diverged(checkpoint.state_rev, checkpoint.round_index, detail)
                    .with_replayed_bytes(end)
                    .into(),
            );
        }
    }
    Ok(end)
}

fn check_save(pack: &Path, records: &[Record], base: &[u8]) -> Result<(), Error> {
    let live = journal::read_save(&journal::save_path(pack))?;
    let save = load_save_bytes(&live)?;
    if records.is_empty() {
        let canonical = save::emit(&save)?.into_bytes();
        if live != canonical {
            let diverged = Divergence::new(
                DivergenceKind::Save,
                save.state_rev,
                None,
                Reason::HandEdit.as_str(),
            );
            return Err(set_first_diverged_line(
                diverged.with_replayed_bytes(canonical),
                &live,
            ));
        }
    }
    if let Some(checkpoint) = journal::latest_checkpoint(records) {
        if blake2s_tagged(&live) != checkpoint.state_hash {
            let snap = fs::read(journal::checkpoint_path(pack, checkpoint.state_rev)).ok();
            let diverged = Divergence {
                replayed_bytes: snap,
                ..Divergence::new(
                    DivergenceKind::Save,
                    checkpoint.state_rev,
                    Some(checkpoint.round_index),
                    "state hash differed",
                )
            };
            return Err(set_first_diverged_line(diverged, &live));
        }
    } else if live != base {
        let diverged = Divergence::new(
            DivergenceKind::Save,
            save.state_rev,
            None,
            "state hash differed",
        );
        return Err(set_first_diverged_line(
            diverged.with_replayed_bytes(base.to_vec()),
            &live,
        ));
    }
    Ok(())
}

/// Name the first line of `live` that differs from the replayed bytes.
fn set_first_diverged_line(mut diverged: Divergence, live: &[u8]) -> Error {
    diverged.save_line = diverged.replayed_bytes.as_deref().and_then(|replayed| {
        let live_text = std::str::from_utf8(live).ok()?;
        let replayed_text = std::str::from_utf8(replayed).ok()?;
        crate::check::find_first_diverged_line(live_text, replayed_text)
    });
    diverged.into()
}

/// A replay divergence names the first differing line of the save, or of the base without one.
fn read_first_diverged_line(pack: &Path, err: Error) -> Error {
    let Error::Diverged(diverged) = err else {
        return err;
    };
    let live = fs::read(journal::save_path(pack))
        .or_else(|_| fs::read(journal::base_path(pack)))
        .unwrap_or_default();
    set_first_diverged_line(diverged, &live)
}

fn read_base(pack: &Path) -> Result<Vec<u8>, Error> {
    let base = journal::base_path(pack);
    if base.exists() {
        return Ok(journal::read_save(&base)?);
    }
    Ok(journal::read_save(&journal::save_path(pack))?)
}

fn replay(pack: &Path, start: &[u8], entries: &[Record]) -> Result<Vec<u8>, Error> {
    let mut session = replay::open_at(pack, start)?;
    let mut changed = false;
    for entry in entries {
        replay::apply_record(&mut session, entry)?;
        changed |= entry.changes_state();
    }
    if changed {
        emit_save_bytes(&session)
    } else {
        Ok(start.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{journal, run, run_quick};
    use crate::testing::copy_pack;
    use crate::testing::play_empty_round;

    #[test]
    fn verify_and_quick_verify_reject_tampered_save_after_checkpoint() {
        let pack = copy_pack("docks", "replay-save-tamper");
        let pack_yaml = pack.join("pack.yaml");
        let mut cfg = fs::read_to_string(&pack_yaml).unwrap();
        cfg.push_str("\ncheckpoint_every: 2\n");
        fs::write(&pack_yaml, cfg).unwrap();
        play_empty_round(&pack);
        play_empty_round(&pack);
        let save = journal::save_path(&pack);
        let text = fs::read_to_string(&save).unwrap();
        let tampered = text.replacen("state_rev: 2", "state_rev: 99", 1);
        assert_ne!(tampered, text);
        fs::write(&save, tampered).unwrap();
        let full_err = run(&pack, None).unwrap_err().to_string();
        assert!(full_err.contains("state_rev 2 save"), "{full_err}");
        assert!(full_err.contains("save.yaml:"), "{full_err}");
        let quick_err = run_quick(&pack).unwrap_err().to_string();
        assert!(quick_err.contains("state_rev 2 save"), "{quick_err}");
        assert!(quick_err.contains("save.yaml:"), "{quick_err}");
        let _ = fs::remove_dir_all(&pack);
    }
}
