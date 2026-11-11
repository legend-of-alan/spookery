//! `story/*.yaml`: rules that belong to no place, cast member or item, and the meters and flags
//! the story carries.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use crate::canon::meters::{add_own_meters, meter_defs, MeterYaml};
use crate::canon::rule_list::{self, RuleYaml};
use crate::canon::source::{
    collect_files, load_tagged, unreadable_dir, Diags, FileText, Locator, STORY_SCHEMA,
};
use crate::canon::StoryDef;
use crate::diagnostic::schema_error;
use crate::entity;
use crate::rules::rule::{Owner, Rule};
use crate::state::component::meter::MeterDef;
use crate::state::id::StoryId;

/// Every `story/*.yaml` file in directory order: its rules compiled, and its meters added to
/// `meters` when those loaded. A file with an empty id, or an id an earlier file has, is refused
/// and skipped. A pack without `story/` has none.
pub(super) fn load_stories(
    root: &Path,
    mut meters: Option<&mut BTreeMap<String, MeterDef>>,
    world_meters: Option<&BTreeMap<String, MeterDef>>,
    diags: &mut Diags,
) -> (Vec<Rule>, BTreeMap<StoryId, StoryDef>) {
    let dir = root.join("story");
    let mut files = Vec::new();
    if dir.is_dir() && collect_files(&dir, root, &mut files).is_err() {
        diags.push(unreadable_dir("story"));
    }
    let mut rules = Vec::new();
    let mut stories = BTreeMap::new();
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
        let Some(story) = diags.keep(load_tagged::<StoryDoc>(&file, text, STORY_SCHEMA, 0)) else {
            continue;
        };
        let source = FileText {
            file: &file,
            text,
            line_base: 0,
        };
        let empty = ("id is empty", "set id to a short story id");
        let twice = ("story", "is already defined", "keep one story with this id");
        let taken = stories.contains_key(story.id.as_str());
        let refused = source
            .refuse_empty(&story.id, "id", Locator::Key("id:"), empty)
            .or_else(|| {
                taken.then(|| source.refuse_duplicate(&story.id, "id", Locator::Key("id:"), twice))
            });
        if let Some(diag) = refused {
            diags.push(diag);
            continue;
        }
        let id: StoryId = story.id.clone().into();
        let carrier = Some(entity::Id::Story(id.clone()));
        let own = diags.keep(meter_defs(&source, story.meters, carrier));
        if let (Some(meters), Some(own)) = (meters.as_deref_mut(), own) {
            add_own_meters(meters, own, world_meters, &source, diags);
        }
        stories.insert(id, StoryDef { flags: story.flags });
        let compiled = rule_list::compile(&file, story.rules, &Owner::Story(story.id));
        rules.extend(diags.keep(compiled).unwrap_or_default());
    }
    (rules, stories)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoryDoc {
    #[serde(rename = "schema")]
    _schema: String,
    id: String,
    #[serde(default)]
    flags: Vec<String>,
    /// The meters only this story carries.
    #[serde(default)]
    meters: Vec<MeterYaml>,
    #[serde(default)]
    rules: Vec<RuleYaml>,
}
