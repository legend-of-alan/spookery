//! `rules/checks.yaml`: the dice checks rules may roll.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::canon::source::{load_headed, At, Diags, Problem, Source, CHECKS_SCHEMA};
use crate::check::CheckDef;
use crate::diagnostic::Diagnostic;

pub(super) fn load_checks(text: &str) -> Result<BTreeMap<String, CheckDef>, Vec<Diagnostic>> {
    const CHECKS_FILE: &str = "rules/checks.yaml";
    let file: ChecksFile = load_headed(CHECKS_FILE, text, CHECKS_SCHEMA, 0)?;
    let source = Source {
        file: CHECKS_FILE,
        text,
        line_base: 0,
    };
    let mut diags = Diags::default();
    let mut checks = BTreeMap::new();
    for (row, entry) in file.checks.into_iter().enumerate() {
        let path = format!("checks[{row}]");
        let mut found = Vec::new();
        let id_path = format!("{path}.id");
        let id_at = At::Row { row, key: "id:" };
        let empty = ("a check id is empty", "set id to a short name");
        found.extend(source.required(&entry.id, &id_path, id_at, empty));
        if entry.dice.count == 0 {
            found.push(
                Problem {
                    source: &source,
                    path: &format!("{path}.dice.count"),
                    at: At::Row { row, key: "count:" },
                    problem: format!("check '{}' rolls zero dice", entry.id),
                    hint: "set dice.count to 1 or more",
                }
                .schema(),
            );
        }
        if entry.dice.sides == 0 {
            found.push(
                Problem {
                    source: &source,
                    path: &format!("{path}.dice.sides"),
                    at: At::Row { row, key: "sides:" },
                    problem: format!("check '{}' has a die with zero sides", entry.id),
                    hint: "set dice.sides to 2 or more",
                }
                .schema(),
            );
        }
        if entry.stat.is_empty() {
            found.push(
                Problem {
                    source: &source,
                    path: &format!("{path}.stat"),
                    at: At::Row { row, key: "stat:" },
                    problem: format!("check '{}' names no meter", entry.id),
                    hint: "set stat to a meter id from rules/meters.yaml",
                }
                .schema(),
            );
        }
        let twice = ("check", "is listed twice", "keep one check");
        // A repeat is checked only on a row with nothing else wrong.
        let taken = found.is_empty() && checks.contains_key(&entry.id);
        found.extend(source.unique(taken, &entry.id, &id_path, id_at, twice));
        if !found.is_empty() {
            diags.extend(found);
            continue;
        }
        checks.insert(
            entry.id,
            CheckDef {
                dice_count: entry.dice.count,
                dice_sides: entry.dice.sides,
                stat: entry.stat,
            },
        );
    }
    diags.result(checks)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChecksFile {
    #[serde(rename = "schema")]
    _schema: String,
    checks: Vec<CheckEntryFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckEntryFile {
    id: String,
    dice: CheckDiceFile,
    stat: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckDiceFile {
    count: u32,
    sides: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canon::tests::rejects;

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
    fn a_check_refusal_names_the_fix() {
        let duplicate = "\
schema: checks
checks:
  - id: grit
    dice:
      count: 2
      sides: 6
    stat: grit
  - id: grit
    dice:
      count: 1
      sides: 6
    stat: grit
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
    stat: grit
";
        let err = load_checks(zero).unwrap_err();
        assert!(err.iter().any(|diag| diag.message.contains("zero dice")));
        assert!(err.iter().any(|diag| diag.message.contains("zero sides")));
    }
}
