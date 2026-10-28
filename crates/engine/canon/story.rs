//! `story/*.yaml`: rules that belong to no place, cast member or item.

use serde::Deserialize;

use crate::canon::rule_list::RuleEntry;
use crate::canon::source::{load_headed, STORY_SCHEMA};
use crate::diagnostic::Diagnostic;

pub(super) fn load_story(file: &str, text: &str) -> Result<StoryFile, Vec<Diagnostic>> {
    load_headed(file, text, STORY_SCHEMA, 0)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StoryFile {
    #[serde(rename = "schema")]
    _schema: String,
    pub(super) id: String,
    #[serde(default)]
    pub(super) rules: Vec<RuleEntry>,
}
