//! `rules/checks.yaml`: the dice checks rules may roll.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::canon::source::{load_tagged, Diags, FileText, Locator, Problem, CHECKS_SCHEMA};
use crate::diagnostic::Diagnostic;
use crate::rules::roll::Check;

pub(super) fn load_checks(
    rel: &str,
    text: &str,
) -> Result<BTreeMap<String, Check>, Vec<Diagnostic>> {
    let file: ChecksFile = load_tagged(rel, text, CHECKS_SCHEMA, 0)?;
    let source = FileText {
        file: rel,
        text,
        line_base: 0,
    };
    let mut diags = Diags::default();
    let mut checks = BTreeMap::new();
    for (row, entry) in file.checks.into_iter().enumerate() {
        let path = format!("checks[{row}]");
        let mut found = Vec::new();
        let id_path = format!("{path}.id");
        let id_at = Locator::Row {
            list: "checks:",
            row,
            key: "id:",
        };
        let empty = ("a check id is empty", "set id to a short name");
        found.extend(source.refuse_empty(&entry.id, &id_path, id_at, empty));
        if entry.dice.count == 0 {
            found.push(
                Problem {
                    source: &source,
                    path: &format!("{path}.dice.count"),
                    at: Locator::Row {
                        list: "checks:",
                        row,
                        key: "count:",
                    },
                    message: format!("check '{}' rolls zero dice", entry.id),
                    hint: "set dice.count to 1 or more",
                }
                .into_diagnostic(),
            );
        }
        if entry.dice.sides == 0 {
            found.push(
                Problem {
                    source: &source,
                    path: &format!("{path}.dice.sides"),
                    at: Locator::Row {
                        list: "checks:",
                        row,
                        key: "sides:",
                    },
                    message: format!("check '{}' has a die with zero sides", entry.id),
                    hint: "set dice.sides to 2 or more",
                }
                .into_diagnostic(),
            );
        }
        if entry.meter.is_empty() {
            found.push(
                Problem {
                    source: &source,
                    path: &format!("{path}.meter"),
                    at: Locator::Row {
                        list: "checks:",
                        row,
                        key: "meter:",
                    },
                    message: format!("check '{}' names no meter", entry.id),
                    hint: "set meter to a meter id from rules/meters.yaml",
                }
                .into_diagnostic(),
            );
        }
        let twice = ("check", "is listed twice", "keep one check");
        // A repeat is checked only on a row with nothing else wrong.
        let taken = found.is_empty() && checks.contains_key(&entry.id);
        found.extend(taken.then(|| source.refuse_duplicate(&entry.id, &id_path, id_at, twice)));
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
struct ChecksFile {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canon::tests::rejects;

    fn load_checks(text: &str) -> Result<BTreeMap<String, Check>, Vec<Diagnostic>> {
        super::load_checks("rules/checks.yaml", text)
    }

    #[test]
    fn load_checks_rejects_an_unknown_tag_and_the_old_versioned_tag() {
        rejects(
            |text| load_checks(text).map(|_| ()),
            "checks",
            concat!("checks/", "v0"),
            "checks: []\n",
        );
    }

    #[test]
    fn a_check_refusal_names_the_row() {
        let duplicate = "\
schema: checks
checks:
  - id: grit
    dice:
      count: 2
      sides: 6
    meter: grit
  - id: grit
    dice:
      count: 1
      sides: 6
    meter: grit
";
        let err = load_checks(duplicate).unwrap_err();
        assert!(
            err.iter().any(|diag| {
                diag.message.contains("listed twice")
                    && diag.path.as_deref() == Some("checks[1].id")
                    && diag.line.is_some()
            }),
            "{err:?}"
        );
        let zero = "\
schema: checks
checks:
  - id: grit
    dice:
      count: 0
      sides: 0
    meter: grit
";
        let err = load_checks(zero).unwrap_err();
        assert!(err.iter().any(|diag| diag.message.contains("zero dice")));
        assert!(err.iter().any(|diag| diag.message.contains("zero sides")));
    }
}
