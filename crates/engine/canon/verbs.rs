//! `rules/verbs.yaml`: the shared verb rules.

use serde::Deserialize;

use crate::canon::rule_list::{self, parse::RuleYaml};
use crate::diagnostic::{parse_tagged, Diagnostic};
use crate::rules::rule::{Rule, Scope};

const SCHEMA: &str = "verbs";

pub(super) fn load(file: &str, text: &str) -> Result<Vec<Rule>, Vec<Diagnostic>> {
    let yaml: Yaml = parse_tagged(file, text, SCHEMA, true)?;
    rule_list::compile::build_rules(file, yaml.rules, &Scope::Verb)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Yaml {
    #[serde(rename = "schema")]
    _schema: String,
    #[serde(default)]
    rules: Vec<RuleYaml>,
}
