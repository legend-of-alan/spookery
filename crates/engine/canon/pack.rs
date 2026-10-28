//! `pack.yaml`: the pack's id, title, system and start.

use serde::Deserialize;

use crate::canon::source::{load_headed, At, Source, PACK_SCHEMA};
use crate::diagnostic::{schema as schema_error, Diagnostic};

pub(super) fn load_pack(text: &str) -> Result<PackFile, Vec<Diagnostic>> {
    let file: PackFile = load_headed("pack.yaml", text, PACK_SCHEMA, 0)?;
    let empty = ("id is empty", "set id to a short pack id");
    let refused = Source::named("pack.yaml").required(&file.id, "id", At::Nowhere, empty);
    refused.map_or(Ok(file), |diag| Err(vec![diag]))
}

impl PackFile {
    /// Ticks between checkpoints: 100 when unset, refused when 0.
    pub(super) fn checkpoint_every(&self) -> Result<u64, Vec<Diagnostic>> {
        match self.checkpoint_every {
            None => Ok(100),
            Some(0) => Err(vec![schema_error(
                "pack.yaml",
                "checkpoint_every is 0",
                "set checkpoint_every to a positive number of ticks, or omit it",
            )
            .with_path("checkpoint_every")]),
            Some(every) => Ok(every),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PackFile {
    #[serde(rename = "schema")]
    _schema: String,
    pub(super) id: String,
    pub(super) title: String,
    pub(super) system: String,
    pub(super) start: PackStart,
    checkpoint_every: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PackStart {
    pub(super) place: String,
    pub(super) pc: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canon::tests::rejects;

    #[test]
    fn load_pack_rejects_an_unknown_tag_and_the_old_versioned_tag() {
        rejects(
            |text| load_pack(text).map(|_| ()),
            "pack",
            concat!("pack/", "v0"),
            "id: ex\ntitle: Ex\nsystem: ex\nstart:\n  place: gate\n  pc: pc_rowan\n",
        );
    }
}
