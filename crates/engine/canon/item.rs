//! `world/items/*.md`: one item.

use serde::Deserialize;

use crate::canon::rule_list::RuleEntry;
use crate::canon::source::{load_headed, ITEM_SCHEMA};
use crate::diagnostic::Diagnostic;

pub(super) fn load_item(file: &str, yaml: &str) -> Result<ItemFile, Vec<Diagnostic>> {
    load_headed(file, yaml, ITEM_SCHEMA, 1)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ItemFile {
    #[serde(rename = "schema")]
    _schema: String,
    pub(super) id: String,
    #[serde(default)]
    pub(super) name: String,
    #[serde(default)]
    pub(super) tags: Vec<String>,
    #[serde(default)]
    pub(super) rules: Vec<RuleEntry>,
}
