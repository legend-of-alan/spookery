//! `story/*.yaml`: rules that belong to no place, cast member or item, and the meters the story
//! carries.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use crate::canon::meters::{self, MeterYaml};
use crate::canon::pack_files::{collect_files, refuse_unreadable_dir};
use crate::canon::refusal::{Diagnostics, FileText, Locator};
use crate::canon::rule_list::{self, parse::RuleYaml};
use crate::diagnostic::{parse_tagged, schema_error};
use crate::ecs::component::meter;
use crate::ecs::id::StoryId;
use crate::entity;
use crate::rules::rule::{Rule, Scope};

const SCHEMA: &str = "story";

#[derive(Clone, Debug)]
pub(crate) struct Def {
    /// The name a player reads, the id when the pack gives none.
    pub name: String,
}

/// Every `story/*.yaml` file in directory order: its rules compiled, and its meters added to
/// `meters` when those loaded. A file with an empty id, or an id an earlier file has, is refused
/// and skipped. A pack without `story/` has none.
pub(super) fn load_dir(
    root: &Path,
    mut meters: Option<&mut BTreeMap<String, meter::Def>>,
    world_meters: Option<&BTreeMap<String, meter::Def>>,
    diagnostics: &mut Diagnostics,
) -> (Vec<Rule>, BTreeMap<StoryId, Def>) {
    let dir = root.join("story");
    let mut files = Vec::new();
    if dir.is_dir() && collect_files(&dir, root, &mut files).is_err() {
        diagnostics.push(refuse_unreadable_dir("story"));
    }
    let mut rules = Vec::new();
    let mut stories = BTreeMap::new();
    for (file, bytes) in files {
        if !file.ends_with(".yaml") {
            continue;
        }
        let Ok(text) = std::str::from_utf8(&bytes) else {
            diagnostics.push(schema_error(
                &file,
                "this file is not UTF-8 text",
                "save it as UTF-8",
            ));
            continue;
        };
        let Some(yaml) = diagnostics.keep(parse_tagged::<Yaml>(&file, text, SCHEMA, true)) else {
            continue;
        };
        let file_text = FileText { file: &file, text };
        let empty_words = ("id is empty", "set id to a short story id");
        let duplicate_words = ("story", "is already defined", "keep one story with this id");
        let is_duplicate = stories.contains_key(yaml.id.as_str());
        let at = Locator::Key("id:");
        let diagnostic = file_text
            .refuse_empty(&yaml.id, "id", at, empty_words)
            .or_else(|| {
                is_duplicate
                    .then(|| file_text.refuse_duplicate(&yaml.id, "id", at, duplicate_words))
            });
        if let Some(diagnostic) = diagnostic {
            diagnostics.push(diagnostic);
            continue;
        }
        let id: StoryId = yaml.id.clone().into();
        let entity_id = Some(entity::Id::Story(id.clone()));
        let own = diagnostics.keep(meters::compile(&file_text, yaml.meters, entity_id));
        if let (Some(meters), Some(own)) = (meters.as_deref_mut(), own) {
            meters::add_own(meters, own, world_meters, &file_text, diagnostics);
        }
        let name = yaml.name.unwrap_or_else(|| yaml.id.clone());
        stories.insert(id, Def { name });
        let compiled = rule_list::compile(&file, yaml.rules, &Scope::Story(yaml.id));
        rules.extend(diagnostics.keep(compiled).unwrap_or_default());
    }
    (rules, stories)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Yaml {
    #[serde(rename = "schema")]
    _schema: String,
    id: String,
    name: Option<String>,
    /// The meters only this story carries.
    #[serde(default)]
    meters: Vec<MeterYaml>,
    #[serde(default)]
    rules: Vec<RuleYaml>,
}
