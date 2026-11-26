//! `pack.yaml`: the pack's id, title, system, start and the GM's tone and flavor.

use serde::Deserialize;

use crate::canon::refusal::{FileText, Locator};
use crate::diagnostic::{parse_tagged, schema_error, Diagnostic};

const SCHEMA: &str = "pack";

pub(super) fn load(text: &str) -> Result<Yaml, Vec<Diagnostic>> {
    let yaml: Yaml = parse_tagged("pack.yaml", text, SCHEMA, false)?;
    let empty_words = ("id is empty", "set id to a short pack id");
    let diagnostic = FileText::without_text("pack.yaml").refuse_empty(
        &yaml.id,
        "id",
        Locator::Nowhere,
        empty_words,
    );
    diagnostic.map_or(Ok(yaml), |diagnostic| Err(vec![diagnostic]))
}

impl Yaml {
    /// Rounds between checkpoints: 100 when unset, refused when 0.
    pub(super) fn checkpoint_every(&self) -> Result<u64, Vec<Diagnostic>> {
        match self.checkpoint_every {
            None => Ok(100),
            Some(0) => Err(vec![schema_error(
                "pack.yaml",
                "checkpoint_every is 0",
                "set checkpoint_every to a positive number of rounds, or omit it",
            )
            .at_key_path("checkpoint_every")]),
            Some(every) => Ok(every),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Yaml {
    #[serde(rename = "schema")]
    _schema: String,
    pub(super) id: String,
    pub(super) title: String,
    pub(super) system: String,
    pub(super) start: StartYaml,
    checkpoint_every: Option<u64>,
    #[serde(default)]
    pub(super) gm_notes: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StartYaml {
    pub(super) place: String,
}
