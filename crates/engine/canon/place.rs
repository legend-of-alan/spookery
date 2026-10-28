//! `world/places/*.md`: a place and its exits.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

use crate::canon::fact::{fact_ids_of, insert_fact, FactFile};
use crate::canon::rule_list::{self, RuleEntry};
use crate::canon::source::{load_headed, At, Diags, Page, Problem, Source, PLACE_SCHEMA};
use crate::canon::{Exit, FactCanon, PlaceCanon};
use crate::diagnostic::{schema as schema_error, Diagnostic};
use crate::rule::{Cost, Owner, Rule};

/// One place page into `places` and its facts into `facts`, and its rules. A bad fact
/// stops the file; the exits compile before the duplicate id check.
pub(super) fn add_place(
    page: Page,
    facts: &mut BTreeMap<String, FactCanon>,
    places: &mut BTreeMap<String, PlaceCanon>,
    diags: &mut Diags,
) -> Option<Vec<Rule>> {
    let file = diags.keep(load_place(&page.file, &page.yaml, 1))?;
    let mut fact_ok = true;
    for (index, fact) in file.facts.into_iter().enumerate() {
        let inserted = insert_fact(facts, &page.file, &page.text, index, fact, &file.id);
        fact_ok &= diags.keep(inserted).is_some();
    }
    if !fact_ok {
        return None;
    }
    let ids = fact_ids_of(facts, &file.id);
    let exits = diags
        .keep(compile_exits(&page.file, &file.exits))
        .unwrap_or_default();
    if places.contains_key(&file.id) {
        diags.push(
            schema_error(
                &page.file,
                &format!("place '{}' is already defined", file.id),
                "keep one place with this id",
            )
            .with_path("id"),
        );
        return None;
    }
    places.insert(
        file.id.clone(),
        PlaceCanon {
            name: file.display_name,
            facts: ids,
            body: page.body,
            exits,
        },
    );
    diags.keep(rule_list::compile(
        &page.file,
        file.rules,
        &Owner::Place(file.id),
    ))
}

fn load_place(file: &str, yaml: &str, line_base: u32) -> Result<PlaceFile, Vec<Diagnostic>> {
    let parsed: PlaceFile = load_headed(file, yaml, PLACE_SCHEMA, line_base)?;
    let source = Source {
        file,
        text: yaml,
        line_base,
    };
    let mut diags = Diags::default();
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
    diags.result(parsed)
}

fn compile_exits(file: &str, exits: &[ExitFile]) -> Result<Vec<Exit>, Vec<Diagnostic>> {
    let source = Source::named(file);
    let mut diags = Diags::default();
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
    diags.result(compiled)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlaceFile {
    #[serde(rename = "schema")]
    _schema: String,
    id: String,
    display_name: String,
    facts: Vec<FactFile>,
    #[serde(default)]
    exits: Vec<ExitFile>,
    #[serde(default)]
    rules: Vec<RuleEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExitFile {
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
