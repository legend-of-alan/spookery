//! Check definitions, and the band a roll total lands in.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::diagnostic::{locate_row_field, parse_file, schema, tag_diagnostic, Diagnostic};

const CHECKS_FILE: &str = "rules/checks.yaml";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckDef {
    pub dice_count: u32,
    pub dice_sides: u32,
    pub stat: String,
}

/// One roll's band. A row without `partial_at` is `Success` or `Miss`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Success,
    Partial,
    Miss,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Partial => "partial",
            Self::Miss => "miss",
        }
    }
}

/// `total` against `target`, with an optional lower band.
/// `total >= target` is success. `partial_at <= total < target` is partial.
/// Anything lower, and any binary roll, is a miss.
pub fn classify(total: i64, target: i64, partial_at: Option<i64>) -> Outcome {
    if total >= target {
        Outcome::Success
    } else if partial_at.is_some_and(|band| total >= band) {
        Outcome::Partial
    } else {
        Outcome::Miss
    }
}

pub(crate) fn load_checks(text: &str) -> Result<BTreeMap<String, CheckDef>, Vec<Diagnostic>> {
    if let Some(diag) = tag_diagnostic(text, crate::format::CHECKS, CHECKS_FILE, 0) {
        return Err(vec![diag]);
    }
    let file: ChecksFile = parse_file(CHECKS_FILE, text, 0)?;
    let mut diags = Vec::new();
    if file.schema != crate::format::CHECKS {
        diags.push(
            schema(
                CHECKS_FILE,
                &format!("schema is '{}', which this file does not use", file.schema),
                "write 'schema: checks' with no version suffix",
            )
            .with_path("schema"),
        );
    }
    let mut checks = BTreeMap::new();
    for (index, entry) in file.checks.into_iter().enumerate() {
        let path = format!("checks[{index}]");
        let mut bad = false;
        if entry.id.is_empty() {
            diags.push(check_problem(
                text,
                index,
                "id:",
                &format!("{path}.id"),
                "a check id is empty",
                "set id to a short name",
            ));
            bad = true;
        }
        if entry.dice.count == 0 {
            diags.push(check_problem(
                text,
                index,
                "count:",
                &format!("{path}.dice.count"),
                &format!("check '{}' rolls zero dice", entry.id),
                "set dice.count to 1 or more",
            ));
            bad = true;
        }
        if entry.dice.sides == 0 {
            diags.push(check_problem(
                text,
                index,
                "sides:",
                &format!("{path}.dice.sides"),
                &format!("check '{}' has a die with zero sides", entry.id),
                "set dice.sides to 2 or more",
            ));
            bad = true;
        }
        if entry.stat.is_empty() {
            diags.push(check_problem(
                text,
                index,
                "stat:",
                &format!("{path}.stat"),
                &format!("check '{}' names no meter", entry.id),
                "set stat to a meter id from rules/meters.yaml",
            ));
            bad = true;
        }
        if !bad && checks.contains_key(&entry.id) {
            diags.push(check_problem(
                text,
                index,
                "id:",
                &format!("{path}.id"),
                &format!("check '{}' is listed twice", entry.id),
                "keep one check",
            ));
            bad = true;
        }
        if !bad {
            checks.insert(
                entry.id,
                CheckDef {
                    dice_count: entry.dice.count,
                    dice_sides: entry.dice.sides,
                    stat: entry.stat,
                },
            );
        }
    }
    if diags.is_empty() {
        Ok(checks)
    } else {
        Err(diags)
    }
}

fn check_problem(
    text: &str,
    index: usize,
    field: &str,
    path: &str,
    problem: &str,
    hint: &str,
) -> Diagnostic {
    let mut diag = schema(CHECKS_FILE, problem, hint).with_path(path);
    if let Some((line, column)) = locate_row_field(text, "id:", index, field) {
        diag = diag.at_line(line, Some(column));
    }
    diag
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChecksFile {
    schema: String,
    checks: Vec<CheckEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckEntry {
    id: String,
    dice: Dice,
    stat: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Dice {
    count: u32,
    sides: u32,
}

#[cfg(test)]
mod tests {
    use super::{classify, load_checks, Outcome};
    use crate::error::Reason;

    #[test]
    fn classification_meets_every_threshold() {
        let success = Outcome::Success;
        let partial = Outcome::Partial;
        let miss = Outcome::Miss;
        let cases = [
            (10, 10, None, success),
            (9, 10, None, miss),
            (11, 10, None, success),
            (10, 10, Some(7), success),
            (9, 10, Some(7), partial),
            (7, 10, Some(7), partial),
            (6, 10, Some(7), miss),
            (11, 10, Some(7), success),
            (0, 0, None, success),
            (0, 0, Some(-1), success),
            (-1, 0, Some(-1), partial),
            (-2, 0, Some(-1), miss),
        ];
        for (total, target, partial_at, expected) in cases {
            assert_eq!(
                classify(total, target, partial_at),
                expected,
                "{total} vs {target} band {partial_at:?}"
            );
        }
    }

    fn text(tag: &str) -> String {
        format!("schema: {tag}\nchecks: []\n")
    }

    #[test]
    fn an_unknown_tag_and_the_old_versioned_tag_are_schema() {
        use crate::diagnostic::reason_of;
        assert!(load_checks(&text("checks")).is_ok());
        let old = load_checks(&text(concat!("checks/", "v0"))).unwrap_err();
        assert!(old[0].message.contains("no version"), "{old:?}");
        assert_eq!(reason_of(old), Reason::Schema);
        let bogus = load_checks(&text("bogus")).unwrap_err();
        assert!(bogus[0].message.contains("does not use"), "{bogus:?}");
        assert_eq!(reason_of(bogus), Reason::Schema);
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
