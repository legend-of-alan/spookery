//! `world/items/*.md`: one item.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::canon::rule_list::{self, RuleEntry};
use crate::canon::source::{load_headed, Diags, Page, ITEM_SCHEMA};
use crate::canon::ItemCanon;
use crate::diagnostic::schema as schema_error;
use crate::rule::{Owner, Rule};

/// One item page into `items`, and its rules. The rules compile before the item is kept.
pub(super) fn add_item(
    page: Page,
    items: &mut BTreeMap<String, ItemCanon>,
    diags: &mut Diags,
) -> Option<Vec<Rule>> {
    let item: ItemFile = diags.keep(load_headed(&page.file, &page.yaml, ITEM_SCHEMA, 1))?;
    if item.id.is_empty() {
        diags.push(schema_error(
            &page.file,
            "an item id is empty",
            "set id to a short name",
        ));
        return None;
    }
    if items.contains_key(&item.id) {
        diags.push(schema_error(
            &page.file,
            &format!("item '{}' is already defined", item.id),
            "keep one item with this id",
        ));
        return None;
    }
    let rules = rule_list::compile(&page.file, item.rules, &Owner::Item(item.id.clone()));
    items.insert(
        item.id,
        ItemCanon {
            name: item.name,
            tags: item.tags,
            body: page.body.trim().to_string(),
        },
    );
    diags.keep(rules)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ItemFile {
    #[serde(rename = "schema")]
    _schema: String,
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    rules: Vec<RuleEntry>,
}
