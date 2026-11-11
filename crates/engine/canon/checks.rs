//! `rules/checks.yaml`: the dice checks rules may roll.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::canon::source::{load_tagged, Diags, FileText, CHECKS_SCHEMA};
use crate::diagnostic::Diagnostic;
use crate::rules::roll::Check;

pub(super) fn load_checks(
    rel: &str,
    text: &str,
) -> Result<BTreeMap<String, Check>, Vec<Diagnostic>> {
    let doc: ChecksDoc = load_tagged(rel, text, CHECKS_SCHEMA, 0)?;
    let source = FileText {
        file: rel,
        text,
        line_base: 0,
    };
    let mut diags = Diags::default();
    let mut checks = BTreeMap::new();
    for (row, entry) in doc.checks.into_iter().enumerate() {
        let id = &entry.id;
        let rules = [
            (
                entry.dice.count == 0,
                "dice.count",
                format!("check '{id}' rolls zero dice"),
                "set dice.count to 1 or more",
            ),
            (
                entry.dice.sides == 0,
                "dice.sides",
                format!("check '{id}' has a die with zero sides"),
                "set dice.sides to 2 or more",
            ),
            (
                entry.meter.is_empty(),
                "meter",
                format!("check '{id}' names no meter"),
                "set meter to a meter id from rules/meters.yaml",
            ),
        ];
        let found = source.refuse_row("check", row, id, checks.contains_key(id), rules);
        if !found.is_empty() {
            diags.extend(found);
            continue;
        }
        checks.insert(
            entry.id,
            Check {
                dice_count: entry.dice.count,
                dice_sides: entry.dice.sides,
                stat: entry.meter,
            },
        );
    }
    diags.into_result(checks)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChecksDoc {
    #[serde(rename = "schema")]
    _schema: String,
    checks: Vec<CheckYaml>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckYaml {
    id: String,
    dice: DiceYaml,
    meter: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DiceYaml {
    count: u32,
    sides: u32,
}
