//! `world/places/*.md`: a place and its exits.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

use crate::canon::fact::{FactYaml, Facts};
use crate::canon::rule_list::{self, RuleYaml};
use crate::canon::source::{load_tagged, Diags, FileText, Locator, Page, PLACE_SCHEMA};
use crate::canon::{Exit, PlaceDef};
use crate::diagnostic::{schema_error, Diagnostic};
use crate::rule::{Cost, Owner, Rule};

/// One place page into `places` and its facts into `facts`, and its rules. A bad fact
/// stops the file; the exits compile before the duplicate id check.
pub(super) fn add_place(
    page: Page,
    facts: &mut Facts,
    places: &mut BTreeMap<String, PlaceDef>,
    diags: &mut Diags,
) -> Option<Vec<Rule>> {
    let file = diags.keep(load_place(&page.file, &page.yaml, 1))?;
    let whole = FileText {
        file: &page.file,
        text: &page.text,
        line_base: 0,
    };
    let ids = diags.keep(facts.add(&whole, file.facts, &file.id))?;
    let exits = diags
        .keep(compile_exits(&page.file, &file.exits))
        .unwrap_or_default();
    let twice = ("place", "is already defined", "keep one place with this id");
    let taken = places.contains_key(&file.id);
    if let Some(diag) =
        taken.then(|| whole.refuse_duplicate(&file.id, "id", Locator::Nowhere, twice))
    {
        diags.push(diag);
        return None;
    }
    places.insert(
        file.id.clone(),
        PlaceDef {
            display_name: file.display_name,
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
    let parsed: PlaceFile = load_tagged(file, yaml, PLACE_SCHEMA, line_base)?;
    let source = FileText {
        file,
        text: yaml,
        line_base,
    };
    let mut diags = Diags::default();
    let empty = ("id is empty", "set id to a short place id");
    diags.extend(source.refuse_empty(&parsed.id, "id", Locator::Key("id:"), empty));
    diags.extend(source.refuse_empty_display_name(&parsed.display_name));
    diags.into_result(parsed)
}

fn compile_exits(file: &str, exits: &[ExitYaml]) -> Result<Vec<Exit>, Vec<Diagnostic>> {
    let source = FileText::named(file);
    let mut diags = Diags::default();
    let mut compiled = Vec::new();
    let mut names = BTreeSet::new();
    for (index, exit) in exits.iter().enumerate() {
        if exit.name.is_empty() || exit.to.is_empty() {
            diags.push(
                schema_error(file, "an exit needs a name and a to", "set both")
                    .at_path(format!("exits[{index}]")),
            );
            continue;
        }
        let path = format!("exits[{index}].name");
        let twice = ("exit", "is listed twice", "keep one exit with this name");
        let taken = !names.insert(exit.name.clone());
        if let Some(diag) =
            taken.then(|| source.refuse_duplicate(&exit.name, &path, Locator::Nowhere, twice))
        {
            diags.push(diag);
            continue;
        }
        let cost = match &exit.cost {
            Some(yaml) => match rule_list::cost_of(yaml) {
                Ok(cost) => cost,
                Err(fault) => {
                    diags.push(source.fault(
                        &format!("exits[{index}].cost"),
                        fault,
                        "fix the exit cost",
                    ));
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
                    diags.push(source.fault(
                        &format!("exits[{index}].if"),
                        fault,
                        "fix the exit condition",
                    ));
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
            cost,
            if_,
            locked_text: exit.locked_text.clone(),
            hidden: exit.hidden,
            revealed_by: exit.revealed_by.clone(),
            file: file.to_string(),
            index,
        });
    }
    diags.into_result(compiled)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlaceFile {
    #[serde(rename = "schema")]
    _schema: String,
    id: String,
    display_name: String,
    facts: Vec<FactYaml>,
    #[serde(default)]
    exits: Vec<ExitYaml>,
    #[serde(default)]
    rules: Vec<RuleYaml>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExitYaml {
    to: String,
    name: String,
    cost: Option<rule_list::CostYaml>,
    #[serde(default, rename = "if")]
    if_: Vec<rule_list::CondYaml>,
    #[serde(default)]
    locked_text: String,
    #[serde(default)]
    hidden: bool,
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
