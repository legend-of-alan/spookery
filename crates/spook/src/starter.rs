use std::fs;
use std::io;
use std::path::Path;

/// Files of the starter pack, in write order. `pack_rev` is filled in after
/// these bytes are on disk. The hash is not stored in this source.
const FILES: &[(&str, &str)] = &[
    ("pack.yaml", include_str!("../templates/starter/pack.yaml")),
    (
        "world/places/yard.md",
        include_str!("../templates/starter/world/places/yard.md"),
    ),
    (
        "world/places/lane.md",
        include_str!("../templates/starter/world/places/lane.md"),
    ),
    (
        "cast/keeper.md",
        include_str!("../templates/starter/cast/keeper.md"),
    ),
    (
        "cast/pc-ada.md",
        include_str!("../templates/starter/cast/pc-ada.md"),
    ),
    (
        "rules/moves.yaml",
        include_str!("../templates/starter/rules/moves.yaml"),
    ),
    (
        "rules/checks.yaml",
        include_str!("../templates/starter/rules/checks.yaml"),
    ),
    (
        "rules/meters.yaml",
        include_str!("../templates/starter/rules/meters.yaml"),
    ),
    (
        "rules/tick.yaml",
        include_str!("../templates/starter/rules/tick.yaml"),
    ),
    (
        "state/now.yaml",
        include_str!("../templates/starter/state/now.yaml"),
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
        fs::create_dir(dir).map_err(StarterError::Io)?;
        true
    };

    if let Err(err) = write_files(dir) {
        if created {
            let _ = fs::remove_dir_all(dir);
        } else {
            for (rel, _) in FILES {
                let _ = fs::remove_file(dir.join(rel));
            }
        }
        return Err(err);
    }
    Ok(next_steps(dir))
}

fn is_empty(dir: &Path) -> Result<bool, StarterError> {
    let mut entries = fs::read_dir(dir).map_err(StarterError::Io)?;
    Ok(entries.next().is_none())
}

fn write_files(dir: &Path) -> Result<(), StarterError> {
    for (rel, text) in FILES {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(StarterError::Io)?;
        }
        fs::write(&path, text.as_bytes()).map_err(StarterError::Io)?;
    }
    restamp(dir)
}

fn restamp(dir: &Path) -> Result<(), StarterError> {
    let rev = spookery_kernel::canon::pack_rev(dir)
        .map_err(|_| StarterError::Message("the starter canon could not be hashed".to_string()))?;
    let path = dir.join("state/now.yaml");
    let text = fs::read_to_string(&path).map_err(StarterError::Io)?;
    let mut now = spookery_kernel::codec::load_now(&text)
        .map_err(|_| StarterError::Message("the starter save could not be read".to_string()))?;
    now.pack_rev = rev;
    let emitted = spookery_kernel::codec::emit_now(&now)
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
    #[test]
    fn the_generator_does_not_store_a_hash() {
        let src = include_str!("starter.rs");
        let needle = ["blake2s", ":"].concat();
        assert!(
            !src.contains(&needle),
            "pack_rev is computed when the pack is written"
        );
    }
}
