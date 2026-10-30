//! `facts:` in a place file.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::canon::source::{Diags, FileText, Locator};
use crate::canon::FactDef;
use crate::diagnostic::Diagnostic;

/// Every fact in the pack by id, each owned by one place.
#[derive(Default)]
pub(super) struct Facts(pub(super) BTreeMap<String, FactDef>);

impl Facts {
    /// One place file's facts, or the refusals. `source` is the whole file, so a
    /// refusal's line counts from its first line.
    pub fn add(
        &mut self,
        source: &FileText,
        facts: Vec<FactYaml>,
        place: &str,
    ) -> Result<(), Vec<Diagnostic>> {
        let mut diags = Diags::default();
        for (row, fact) in facts.into_iter().enumerate() {
            diags.extend(self.insert(source, row, fact, place));
        }
        diags.into_result(())
    }

    fn insert(
        &mut self,
        source: &FileText,
        row: usize,
        FactYaml { id, text, hidden }: FactYaml,
        place: &str,
    ) -> Option<Diagnostic> {
        let path = format!("facts[{row}].id");
        let at = Locator::Row {
            list: "facts:",
            row,
            key: "id:",
        };
        let empty = ("a fact id is empty", "set id to a short name");
        let twice = ("fact", "is already defined", "keep one fact with this id");
        let taken = self.0.contains_key(&id);
        let refused = source
            .refuse_empty(&id, &path, at, empty)
            .or_else(|| taken.then(|| source.refuse_duplicate(&id, &path, at, twice)));
        if refused.is_none() {
            let place_id = place.to_string();
            self.0.insert(
                id,
                FactDef {
                    text,
                    hidden,
                    place_id,
                },
            );
        }
        refused
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FactYaml {
    pub(super) id: String,
    pub(super) text: String,
    #[serde(default)]
    pub(super) hidden: bool,
}
