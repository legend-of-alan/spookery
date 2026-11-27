//! The read-only check of a whole pack that `spook check` runs.

use std::path::Path;
use std::sync::Arc;

use crate::canon;
use crate::diagnostic::{error, Diagnostic};
use crate::ecs;
use crate::store::journal;
use crate::store::save;
use crate::Reason;

/// Read the pack and report every loader and lint diagnostic. Writes nothing.
pub fn run(root: &Path) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let canon = match canon::load(root) {
        Ok(canon) => Some(Arc::new(canon)),
        Err(more) => {
            diagnostics.extend(more);
            None
        }
    };
    let mut save = None;
    match journal::read_save(&journal::save_path(root)) {
        Ok(bytes) => match save::load(root, canon.as_ref(), &bytes) {
            Ok(loaded) => save = Some(loaded),
            Err(more) => diagnostics.extend(more),
        },
        Err(Reason::NotInSpec) => {}
        Err(reason) => diagnostics.push(
            error(
                "state/save.yaml",
                reason,
                "state/save.yaml could not be read",
                "check that the file is readable",
            )
            .at_key_path("schema"),
        ),
    }
    if let Some(canon) = &canon {
        let world = save.map(|save| ecs::World::new(save, Arc::clone(canon)));
        let saved_flags = world
            .as_ref()
            .map(save::find_saved_flags)
            .unwrap_or_default();
        diagnostics.extend(crate::canon::lint::run(canon, &saved_flags));
    }
    diagnostics
}
