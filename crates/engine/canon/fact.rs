//! `facts:` in a place or cast file.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::canon::source::{At, Problem, Source};
use crate::canon::FactCanon;
use crate::diagnostic::Diagnostic;

pub(super) fn fact_ids_of(facts: &BTreeMap<String, FactCanon>, owner: &str) -> Vec<String> {
    facts
        .iter()
        .filter(|(_, fact)| fact.owner_id == owner)
        .map(|(id, _)| id.clone())
        .collect()
}

pub(super) fn insert_fact(
    facts: &mut BTreeMap<String, FactCanon>,
    file: &str,
    text: &str,
    row: usize,
    fact: FactFile,
    owner: &str,
) -> Result<(), Vec<Diagnostic>> {
    let source = Source {
        file,
        text,
        line_base: 0,
    };
    let path = format!("facts[{row}].id");
    if fact.id.is_empty() {
        return Err(vec![Problem {
            source: &source,
            path: &path,
            at: At::Row { row, key: "id:" },
            problem: "a fact id is empty".into(),
            hint: "set id to a short name",
        }
        .schema()]);
    }
    if facts.contains_key(&fact.id) {
        return Err(vec![Problem {
            source: &source,
            path: &path,
            at: At::Row { row, key: "id:" },
            problem: format!("fact '{}' is already defined", fact.id),
            hint: "keep one fact with this id",
        }
        .schema()]);
    }
    facts.insert(
        fact.id,
        FactCanon {
            text: fact.text,
            hidden: fact.hidden,
            owner_id: owner.to_string(),
        },
    );
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FactFile {
    pub(super) id: String,
    pub(super) text: String,
    pub(super) hidden: bool,
}
