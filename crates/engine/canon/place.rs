//! `world/places/*.md`: a place and its exits.

use std::collections::BTreeSet;

use serde::Deserialize;

use crate::canon::fact::FactFile;
use crate::canon::rule_list::{self, RuleEntry};
use crate::canon::source::{load_headed, At, Problem, Source, PLACE_SCHEMA};
use crate::canon::Exit;
use crate::diagnostic::{schema as schema_error, Diagnostic};
use crate::rule::Cost;

pub(super) fn load_place(
    file: &str,
    yaml: &str,
    line_base: u32,
) -> Result<PlaceFile, Vec<Diagnostic>> {
    let parsed: PlaceFile = load_headed(file, yaml, PLACE_SCHEMA, line_base)?;
    let source = Source {
        file,
        text: yaml,
        line_base,
    };
    let mut diags = Vec::new();
    if parsed.id.is_empty() {
        diags.push(
            Problem {
                source: &source,
                path: "id",
                at: At::Key("id:"),
                problem: "id is empty".into(),
                hint: "set id to a short place id",
            }
            .schema(),
        );
    }
    if parsed.display_name.trim().is_empty() {
        diags.push(
            Problem {
                source: &source,
                path: "display_name",
                at: At::Key("display_name:"),
                problem: "display_name is empty".into(),
                hint: "set display_name to the name players see",
            }
            .schema(),
        );
    }
    if diags.is_empty() {
        Ok(parsed)
    } else {
        Err(diags)
    }
}

pub(super) fn compile_exits(file: &str, exits: &[ExitFile]) -> Result<Vec<Exit>, Vec<Diagnostic>> {
    let source = Source::named(file);
    let mut diags = Vec::new();
    let mut compiled = Vec::new();
    let mut names = BTreeSet::new();
    for (index, exit) in exits.iter().enumerate() {
        if exit.name.is_empty() || exit.to.is_empty() {
            diags.push(
                schema_error(file, "an exit needs a name and a to", "set both")
                    .with_path(format!("exits[{index}]")),
            );
            continue;
        }
        if !names.insert(exit.name.clone()) {
            diags.push(
                schema_error(
                    file,
                    &format!("exit '{}' is listed twice", exit.name),
                    "keep one exit with this name",
                )
                .with_path(format!("exits[{index}].name")),
            );
            continue;
        }
        let cost = match &exit.cost {
            Some(yaml) => match rule_list::cost_of(yaml) {
                Ok(cost) => cost,
                Err(fault) => {
                    diags.push(
                        Problem {
                            source: &source,
                            path: &format!("exits[{index}].cost"),
                            at: At::Spot(fault.spot),
                            problem: fault.message,
                            hint: "fix the exit cost",
                        }
                        .schema(),
                    );
                    continue;
                }
            },
            None => Cost {
                minutes: 0,
                meters: Vec::new(),
                items: Vec::new(),
            },
        };
        let mut if_ = Vec::new();
        let mut bad = false;
        for cond in &exit.if_ {
            match rule_list::cond_of(cond) {
                Ok(cond) => if_.push(cond),
                Err(fault) => {
                    diags.push(
                        Problem {
                            source: &source,
                            path: &format!("exits[{index}].if"),
                            at: At::Spot(fault.spot),
                            problem: fault.message,
                            hint: "fix the exit condition",
                        }
                        .schema(),
                    );
                    bad = true;
                }
            }
        }
        if bad {
            continue;
        }
        compiled.push(Exit {
            name: exit.name.clone(),
            to: exit.to.clone(),
            minutes: cost.minutes,
            cost,
            if_,
            locked_text: exit.locked_text.clone(),
            hidden: exit.hidden,
            revealed_by: exit.revealed_by.clone(),
            file: file.to_string(),
            index,
        });
    }
    if diags.is_empty() {
        Ok(compiled)
    } else {
        Err(diags)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PlaceFile {
    #[serde(rename = "schema")]
    _schema: String,
    pub(super) id: String,
    pub(super) display_name: String,
    pub(super) facts: Vec<FactFile>,
    #[serde(default)]
    pub(super) exits: Vec<ExitFile>,
    #[serde(default)]
    pub(super) rules: Vec<RuleEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ExitFile {
    to: String,
    name: String,
    #[serde(default)]
    cost: Option<rule_list::CostYaml>,
    #[serde(default, rename = "if")]
    if_: Vec<rule_list::CondYaml>,
    #[serde(default)]
    locked_text: String,
    #[serde(default)]
    hidden: bool,
    #[serde(default)]
    revealed_by: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canon::tests::rejects;

    #[test]
    fn load_place_rejects_an_unknown_tag_and_the_old_versioned_tag() {
        rejects(
            |text| load_place("world/places/gate.md", text, 0).map(|_| ()),
            "place",
            concat!("place/", "v0"),
            "id: gate\ndisplay_name: Gate\nfacts: []\n",
        );
    }
}
