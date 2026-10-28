//! `world/items/*.md`: one item.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::canon::rule_list::{self, RuleEntry};
use crate::canon::source::{load_headed, At, Diags, Page, Source, ITEM_SCHEMA};
use crate::canon::ItemCanon;
use crate::rule::{Owner, Rule};

/// One item page into `items`, and its rules. The item is kept even when its rules do not
/// compile, so a later item with the same id is still refused.
pub(super) fn add_item(
    page: Page,
    items: &mut BTreeMap<String, ItemCanon>,
    diags: &mut Diags,
) -> Option<Vec<Rule>> {
    let item: ItemFile = diags.keep(load_headed(&page.file, &page.yaml, ITEM_SCHEMA, 1))?;
    let source = Source::named(&page.file);
    let empty = ("an item id is empty", "set id to a short name");
    let twice = ("item", "is already defined", "keep one item with this id");
    let taken = items.contains_key(&item.id);
    let refused = source
        .required(&item.id, "", At::Nowhere, empty)
        .or_else(|| taken.then(|| source.repeat(&item.id, "", At::Nowhere, twice)));
    if let Some(diag) = refused {
        diags.push(diag);
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
