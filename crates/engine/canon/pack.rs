//! `pack.yaml`: the pack's id, title, system and start.

use serde::Deserialize;

use crate::canon::source::{load_headed, PACK_SCHEMA};
use crate::diagnostic::{schema as schema_error, Diagnostic};

pub(super) fn load_pack(text: &str) -> Result<PackFile, Vec<Diagnostic>> {
    let file: PackFile = load_headed("pack.yaml", text, PACK_SCHEMA, 0)?;
    if file.id.is_empty() {
        return Err(vec![schema_error(
            "pack.yaml",
            "id is empty",
            "set id to a short pack id",
        )
        .with_path("id")]);
    }
    Ok(file)
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
    #[serde(default)]
    pub(super) checkpoint_every: Option<u64>,
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
