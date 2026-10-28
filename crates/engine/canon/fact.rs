//! `facts:` in a place or cast file.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::canon::source::{At, Diags, Source};
use crate::canon::FactCanon;
use crate::diagnostic::Diagnostic;

/// Every fact in the pack by id, from places and cast alike.
#[derive(Default)]
pub(super) struct Facts(pub(super) BTreeMap<String, FactCanon>);

impl Facts {
    /// One file's facts for `owner`, then the ids of every fact `owner` has, or the
    /// refusals. `source` is the whole file, so its own `id:` is row 0 (issue #328).
    pub fn add(
        &mut self,
        source: &Source,
        facts: Vec<FactFile>,
        owner: &str,
    ) -> Result<Vec<String>, Vec<Diagnostic>> {
        let mut diags = Diags::default();
        for (row, fact) in facts.into_iter().enumerate() {
            diags.extend(self.insert(source, row, fact, owner));
        }
        diags.result(())?;
        Ok(self
            .0
            .iter()
            .filter(|(_, fact)| fact.owner_id == owner)
            .map(|(id, _)| id.clone())
            .collect())
    }

    fn insert(
        &mut self,
        source: &Source,
        row: usize,
        FactFile { id, text, hidden }: FactFile,
        owner: &str,
    ) -> Option<Diagnostic> {
        let path = format!("facts[{row}].id");
        let at = At::Row { row, key: "id:" };
        let empty = ("a fact id is empty", "set id to a short name");
        let twice = ("fact", "is already defined", "keep one fact with this id");
        let taken = self.0.contains_key(&id);
        let refused = source
            .required(&id, &path, at, empty)
            .or_else(|| source.unique(taken, &id, &path, at, twice));
        if refused.is_none() {
            let owner_id = owner.to_string();
            self.0.insert(
                id,
                FactCanon {
                    text,
                    hidden,
                    owner_id,
                },
            );
        }
        refused
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FactFile {
    pub(super) id: String,
    pub(super) text: String,
    pub(super) hidden: bool,
}
