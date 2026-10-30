//! `story/*.yaml`: rules that belong to no place, cast member or item.

use std::path::Path;

use serde::Deserialize;

use crate::canon::rule_list::{self, RuleYaml};
use crate::canon::source::{collect_files, load_tagged, unreadable_dir, Diags, STORY_SCHEMA};
use crate::diagnostic::schema_error;
use crate::rule::{Owner, Rule};

/// Every `story/*.yaml` file in directory order, compiled. A pack without `story/` has none.
pub(super) fn load_stories(root: &Path, diags: &mut Diags) -> Vec<Rule> {
    let dir = root.join("story");
    let mut files = Vec::new();
    if dir.is_dir() && collect_files(&dir, root, &mut files).is_err() {
        diags.push(unreadable_dir("story"));
    }
    let mut rules = Vec::new();
    for (file, bytes) in files {
        if !file.ends_with(".yaml") {
            continue;
        }
        let Ok(text) = std::str::from_utf8(&bytes) else {
            diags.push(schema_error(
                &file,
                "this file is not UTF-8 text",
                "save it as UTF-8",
            ));
            continue;
        };
        let Some(story) = diags.keep(load_tagged::<StoryFile>(&file, text, STORY_SCHEMA, 0)) else {
            continue;
        };
        let compiled = rule_list::compile(&file, story.rules, &Owner::Story(story.id));
        rules.extend(diags.keep(compiled).unwrap_or_default());
    }
    rules
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoryFile {
    #[serde(rename = "schema")]
    _schema: String,
    id: String,
    #[serde(default)]
    rules: Vec<RuleYaml>,
}
