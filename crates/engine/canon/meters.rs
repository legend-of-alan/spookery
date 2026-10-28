//! `rules/meters.yaml`: the meters every actor carries.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::canon::source::{load_headed, At, Problem, Source, METERS_SCHEMA};
use crate::diagnostic::Diagnostic;
use crate::meter::MeterDef;

pub(crate) fn load_meters(text: &str) -> Result<BTreeMap<String, MeterDef>, Vec<Diagnostic>> {
    const FILE: &str = "rules/meters.yaml";
    let file: MetersFile = load_headed(FILE, text, METERS_SCHEMA, 0)?;
    let source = Source {
        file: FILE,
        text,
        line_base: 0,
    };
    let mut diags = Vec::new();
    let mut meters = BTreeMap::new();
    for (row, entry) in file.meters.into_iter().enumerate() {
        let path = format!("meters[{row}]");
        let mut bad = false;
        if entry.id.is_empty() {
            diags.push(
                Problem {
                    source: &source,
                    path: &format!("{path}.id"),
                    at: At::Row { row, key: "id:" },
                    problem: "a meter id is empty".into(),
                    hint: "set id to a short name",
                }
                .schema(),
            );
            bad = true;
        }
        if entry.min > entry.base_cap {
            diags.push(
                Problem {
                    source: &source,
                    path: &format!("{path}.min"),
                    at: At::Row { row, key: "min:" },
                    problem: format!("meter '{}' has min above base_cap", entry.id),
                    hint: "set min at or below base_cap",
                }
                .schema(),
            );
            bad = true;
        }
        if entry.base_cap > entry.max {
            diags.push(
                Problem {
                    source: &source,
                    path: &format!("{path}.base_cap"),
                    at: At::Row {
                        row,
                        key: "base_cap:",
                    },
                    problem: format!("meter '{}' has base_cap above max", entry.id),
                    hint: "set base_cap at or below max",
                }
                .schema(),
            );
            bad = true;
        }
        if !(entry.min..=entry.base_cap).contains(&entry.start) {
            diags.push(
                Problem {
                    source: &source,
                    path: &format!("{path}.start"),
                    at: At::Row { row, key: "start:" },
                    problem: format!("meter '{}' has start outside min and base_cap", entry.id),
                    hint: "set start between min and base_cap",
                }
                .schema(),
            );
            bad = true;
        }
        if !bad && meters.contains_key(&entry.id) {
            diags.push(
                Problem {
                    source: &source,
                    path: &format!("{path}.id"),
                    at: At::Row { row, key: "id:" },
                    problem: format!("meter '{}' is listed twice", entry.id),
                    hint: "keep one meter",
                }
                .schema(),
            );
            bad = true;
        }
        if !bad {
            meters.insert(
                entry.id,
                MeterDef {
                    min: entry.min,
                    base_cap: entry.base_cap,
                    max: entry.max,
                    start: entry.start,
                    hidden: entry.hidden,
                },
            );
        }
    }
    if diags.is_empty() {
        Ok(meters)
    } else {
        Err(diags)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MetersFile {
    #[serde(rename = "schema")]
    _schema: String,
    meters: Vec<MeterEntryFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MeterEntryFile {
    id: String,
    min: i64,
    base_cap: i64,
    max: i64,
    start: i64,
    #[serde(default)]
    hidden: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canon::tests::rejects;
    use crate::diagnostic::reason_of;
    use crate::error::Reason;

    #[test]
    fn load_meters_rejects_a_broken_inequality_a_duplicate_id_and_an_unknown_key() {
        let broken = "\
schema: meters
meters:
  - id: grit
    min: 2
    base_cap: 1
    max: 3
    start: 1
";
        let err = load_meters(broken).unwrap_err();
        assert!(
            err.iter()
                .any(|diag| diag.message.contains("min above base_cap")),
            "{err:?}"
        );
        assert_eq!(reason_of(err), Reason::Schema);
        let duplicate = "\
schema: meters
meters:
  - id: grit
    min: 0
    base_cap: 1
    max: 1
    start: 1
  - id: grit
    min: 0
    base_cap: 1
    max: 1
    start: 0
";
        let err = load_meters(duplicate).unwrap_err();
        assert!(
            err.iter().any(|diag| diag.message.contains("listed twice")),
            "{err:?}"
        );
        assert_eq!(reason_of(err), Reason::Schema);
        let unknown = "\
schema: meters
meters:
  - id: grit
    min: 0
    base_cap: 1
    max: 1
    start: 1
    kind: stat
";
        let err = load_meters(unknown).unwrap_err();
        assert_eq!(err.len(), 1, "{err:?}");
        assert!(err[0].message.contains("unknown field 'kind'"), "{err:?}");
        assert_eq!(reason_of(err), Reason::Schema);
    }

    #[test]
    fn load_meters_rejects_an_unknown_tag_and_the_old_versioned_tag() {
        rejects(
            |text| load_meters(text).map(|_| ()),
            "meters",
            concat!("meters/", "v0"),
            "meters: []\n",
        );
    }
}
