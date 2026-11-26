//! `world/items/*.md`: one item.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::canon::pack_files::Page;
use crate::canon::refusal::{Diagnostics, FileText, Locator};
use crate::canon::rule_list::{self, parse::RuleYaml};
use crate::diagnostic::parse_tagged;
use crate::rules::rule::{Rule, Scope};

const SCHEMA: &str = "item";

#[derive(Clone, Debug)]
pub struct Def {
    /// The name a player reads, the id when the pack gives none.
    pub name: String,
    pub tags: Vec<String>,
    pub body: String,
}

/// One item page into `items`, and its rules. The item is kept even when its rules do not
/// compile, so a later item with the same id is still refused.
pub(super) fn add(
    page: Page,
    items: &mut BTreeMap<String, Def>,
    diagnostics: &mut Diagnostics,
) -> Option<Vec<Rule>> {
    let yaml: Yaml = diagnostics.keep(parse_tagged(&page.file, &page.frontmatter, SCHEMA, true))?;
    let file_text = FileText::without_text(&page.file);
    let empty_words = ("an item id is empty", "set id to a short item id");
    let duplicate_words = ("item", "is already defined", "keep one item with this id");
    let is_duplicate = items.contains_key(&yaml.id);
    let diagnostic = file_text
        .refuse_empty(&yaml.id, "", Locator::Nowhere, empty_words)
        .or_else(|| {
            is_duplicate.then(|| {
                file_text.refuse_duplicate(&yaml.id, "", Locator::Nowhere, duplicate_words)
            })
        });
    if let Some(diagnostic) = diagnostic {
        diagnostics.push(diagnostic);
        return None;
    }
    let rules = rule_list::compile(&page.file, yaml.rules, &Scope::Item(yaml.id.clone()));
    items.insert(
        yaml.id.clone(),
        Def {
            name: yaml.name.unwrap_or(yaml.id),
            tags: yaml.tags,
            body: page.body.trim().to_string(),
        },
    );
    diagnostics.keep(rules)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Yaml {
    #[serde(rename = "schema")]
    _schema: String,
    id: String,
    name: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    rules: Vec<RuleYaml>,
}
