//! The files `spook new` writes.

use std::fs;
use std::io;
use std::path::Path;

/// Files of the starter pack, in write order. `pack_rev` is filled in after
/// these bytes are on disk. The hash is not stored in this source.
const FILES: &[(&str, &str)] = &[
    ("pack.yaml", include_str!("templates/starter/pack.yaml")),
    (
        "world/places/yard.md",
        include_str!("templates/starter/world/places/yard.md"),
    ),
    (
        "world/places/lane.md",
        include_str!("templates/starter/world/places/lane.md"),
    ),
    (
        "cast/keeper.md",
        include_str!("templates/starter/cast/keeper.md"),
    ),
    (
        "cast/pc-ada.md",
        include_str!("templates/starter/cast/pc-ada.md"),
    ),
    (
        "rules/verbs.yaml",
        include_str!("templates/starter/rules/verbs.yaml"),
    ),
    (
        "rules/checks.yaml",
        include_str!("templates/starter/rules/checks.yaml"),
    ),
    (
        "rules/meters.yaml",
        include_str!("templates/starter/rules/meters.yaml"),
    ),
    (
        "rules/tick.yaml",
        include_str!("templates/starter/rules/tick.yaml"),
    ),
    (
        "state/save.yaml",
        include_str!("templates/starter/state/save.yaml"),
    ),
];

pub enum StarterError {
    Message(String),
    Io(io::Error),
}

impl std::fmt::Display for StarterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StarterError::Message(text) => f.write_str(text),
            StarterError::Io(err) => write!(f, "{err}"),
        }
    }
}

/// Write the starter pack into `dir` and return the next-step text.
/// An existing non-empty directory is refused and left untouched.
/// A missing parent is refused and left uncreated.
/// A failed write removes whatever this call created.
pub fn write_starter(dir: &Path) -> Result<String, StarterError> {
    let created = if dir.exists() {
        if !dir.is_dir() {
            return Err(StarterError::Message(format!(
                "{} is not a directory",
                dir.display()
            )));
        }
        if !is_empty(dir)? {
            return Err(StarterError::Message(format!(
                "{} is not empty",
                dir.display()
            )));
        }
        false
    } else {
        if let Some(parent) = dir.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                return Err(StarterError::Message(format!(
                    "{} does not exist",
                    parent.display()
                )));
            }
        }
        fs::create_dir(dir).map_err(StarterError::Io)?;
        true
    };

    if let Err(err) = write_files(dir) {
        undo_write(dir, created);
        return Err(err);
    }
    Ok(next_steps(dir))
}

fn is_empty(dir: &Path) -> Result<bool, StarterError> {
    let mut entries = fs::read_dir(dir).map_err(StarterError::Io)?;
    Ok(entries.next().is_none())
}

#[cfg(test)]
thread_local! {
    static STOP_BEFORE: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

fn stop_for_test(index: usize) -> Result<(), StarterError> {
    #[cfg(test)]
    if STOP_BEFORE.with(|cell| cell.get() == Some(index)) {
        return Err(StarterError::Message(
            "the starter write stopped".to_string(),
        ));
    }
    #[cfg(not(test))]
    let _ = index;
    Ok(())
}

fn write_files(dir: &Path) -> Result<(), StarterError> {
    for (index, (rel, text)) in FILES.iter().enumerate() {
        stop_for_test(index)?;
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(StarterError::Io)?;
        }
        fs::write(&path, text.as_bytes()).map_err(StarterError::Io)?;
    }
    restamp(dir)
}

fn undo_write(dir: &Path, created: bool) {
    if created {
        let _ = fs::remove_dir_all(dir);
        return;
    }
    for (rel, _) in FILES.iter().rev() {
        let _ = fs::remove_file(dir.join(rel));
    }
    let mut dirs = starter_dirs();
    // Deepest first, so an empty child goes before its parent.
    dirs.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    for rel in dirs {
        let _ = fs::remove_dir(dir.join(rel));
    }
}

fn starter_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs = Vec::new();
    for (rel, _) in FILES {
        let mut path = Path::new(rel);
        while let Some(parent) = path.parent() {
            if parent.as_os_str().is_empty() {
                break;
            }
            if !dirs.iter().any(|have| have == parent) {
                dirs.push(parent.to_path_buf());
            }
            path = parent;
        }
    }
    dirs
}

fn restamp(dir: &Path) -> Result<(), StarterError> {
    let rev = spookery_engine::canon::pack_rev(dir)
        .map_err(|_| StarterError::Message("the starter canon could not be hashed".to_string()))?;
    let path = dir.join("state/save.yaml");
    let text = fs::read_to_string(&path).map_err(StarterError::Io)?;
    let mut now = spookery_engine::store::load_save(&text)
        .map_err(|_| StarterError::Message("the starter save could not be read".to_string()))?;
    now.pack_rev = rev;
    let emitted = spookery_engine::store::emit_save(&now)
        .map_err(|_| StarterError::Message("the starter save could not be written".to_string()))?;
    fs::write(path, emitted).map_err(StarterError::Io)?;
    Ok(())
}

fn next_steps(dir: &Path) -> String {
    let path = dir.display();
    format!("Wrote {path}\nspook check {path}\nspook run {path}\nspook play {path}\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StopBefore;

    impl StopBefore {
        fn arm(index: usize) -> Self {
            STOP_BEFORE.with(|cell| cell.set(Some(index)));
            Self
        }
    }

    impl Drop for StopBefore {
        fn drop(&mut self) {
            STOP_BEFORE.with(|cell| cell.set(None));
        }
    }

    fn scratch(label: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "spook-new-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&path);
        path
    }

    #[test]
    fn the_generator_does_not_store_a_hash() {
        let src = include_str!("starter.rs");
        let needle = ["blake2s", ":"].concat();
        assert!(
            !src.contains(&needle),
            "pack_rev is computed when the pack is written"
        );
    }

    fn must_fail(dir: &Path) -> String {
        match write_starter(dir) {
            Ok(next) => panic!("write succeeded: {next}"),
            Err(err) => err.to_string(),
        }
    }

    #[test]
    fn a_failed_write_leaves_an_existing_directory_empty() {
        let dir = scratch("fail");
        fs::create_dir(&dir).unwrap();
        let stop = StopBefore::arm(2);
        let err = must_fail(&dir);
        drop(stop);
        assert_eq!(err, "the starter write stopped");
        assert!(
            fs::read_dir(&dir).unwrap().next().is_none(),
            "directory was not emptied"
        );
        let next = match write_starter(&dir) {
            Ok(next) => next,
            Err(err) => panic!("{err}"),
        };
        assert!(next.contains("Wrote"), "{next}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_write_removes_a_directory_it_created() {
        let dir = scratch("gone");
        let stop = StopBefore::arm(2);
        let err = must_fail(&dir);
        drop(stop);
        assert_eq!(err, "the starter write stopped");
        assert!(!dir.exists(), "{}", dir.display());
    }

    #[test]
    fn a_missing_parent_is_a_sentence() {
        let parent = scratch("missing");
        let dir = parent.join("pack");
        let err = must_fail(&dir);
        assert_eq!(err, format!("{} does not exist", parent.display()));
        assert!(!parent.exists(), "{}", parent.display());
        assert!(!dir.exists(), "{}", dir.display());
    }
}
