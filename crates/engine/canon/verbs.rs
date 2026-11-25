//! `rules/verbs.yaml`: the shared verb rules.

use serde::Deserialize;

use crate::canon::rule_list::{self, parse::RuleYaml};
use crate::canon::source::{load_tagged, VERBS_SCHEMA};
use crate::diagnostic::Diagnostic;
use crate::rules::rule::{Owner, Rule};

pub(in crate::canon) fn load(rel: &str, text: &str) -> Result<Vec<Rule>, Vec<Diagnostic>> {
    let doc: Yaml = load_tagged(rel, text, VERBS_SCHEMA, true)?;
    rule_list::compile(rel, doc.rules, &Owner::Verb)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Yaml {
    #[serde(rename = "schema")]
    _schema: String,
    #[serde(default)]
    rules: Vec<RuleYaml>,
}
