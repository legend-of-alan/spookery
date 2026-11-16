//! `world/places/*.md`: a place, its exits, and the meters and flags it carries.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

use crate::canon::fact::{FactYaml, Facts};
use crate::canon::meters::{meter_defs, MeterYaml};
use crate::canon::rule_list::{self, RuleYaml};
use crate::canon::source::{load_tagged, Diags, FileText, Locator, Page, PLACE_SCHEMA};
use crate::canon::{Exit, PlaceDef};
use crate::diagnostic::{schema_error, Diagnostic};
use crate::ecs::component::meter::MeterDef;
use crate::ecs::id::PlaceId;
use crate::entity;
use crate::rules::rule::{Cost, Owner, Rule};

/// The meters one place file declares, by that file, for `load_canon` to add once the meters
/// every actor carries have loaded.
pub(super) type PlaceMeters = Vec<(String, BTreeMap<String, MeterDef>)>;

/// One place page into `places`, its facts into `facts` and its meters into `meters`, and its
/// rules. A bad fact stops the file; the exits compile before the duplicate id check.
pub(super) fn add_place(
    page: Page,
    facts: &mut Facts,
    places: &mut BTreeMap<PlaceId, PlaceDef>,
    meters: &mut PlaceMeters,
    diags: &mut Diags,
) -> Option<Vec<Rule>> {
    let file = diags.keep(load_place(&page.file, &page.yaml, 1))?;
    let whole = FileText {
        file: &page.file,
        text: &page.text,
        line_base: 0,
    };
    diags.keep(facts.add(&whole, file.facts, &file.id))?;
    let exits = diags
        .keep(compile_exits(&page.file, &file.id, &file.exits))
        .unwrap_or_default();
    let twice = ("place", "is already defined", "keep one place with this id");
    let taken = places.contains_key(file.id.as_str());
    if let Some(diag) =
        taken.then(|| whole.refuse_duplicate(&file.id, "id", Locator::Nowhere, twice))
    {
        diags.push(diag);
        return None;
    }
    let id: PlaceId = file.id.clone().into();
    let carrier = Some(entity::Id::Place(id.clone()));
    if let Some(own) = diags.keep(meter_defs(&whole, file.meters, carrier)) {
        meters.push((page.file.clone(), own));
    }
    places.insert(
        id,
        PlaceDef {
            name: file.name.unwrap_or_else(|| file.id.clone()),
            body: page.body,
            exits,
            flags: file.flags,
        },
    );
    diags.keep(rule_list::compile(
        &page.file,
        file.rules,
        &Owner::Place(file.id),
    ))
}

/// Name each exit the pack leaves unnamed: an exit whose id is the place it goes to takes that
/// place's name, and any other takes its id.
pub(super) fn name_exits(places: &mut BTreeMap<PlaceId, PlaceDef>) {
    let place_names: BTreeMap<PlaceId, String> = (places.iter())
        .map(|(id, place)| (id.clone(), place.name.clone()))
        .collect();
    for exit in places.values_mut().flat_map(|place| place.exits.iter_mut()) {
        if exit.name.is_empty() {
            let destination = place_names
                .get(exit.to.as_str())
                .filter(|_| exit.id == exit.to);
            exit.name = destination.unwrap_or(&exit.id).clone();
        }
    }
}

fn load_place(file: &str, yaml: &str, line_base: u32) -> Result<PlaceDoc, Vec<Diagnostic>> {
    let parsed: PlaceDoc = load_tagged(file, yaml, PLACE_SCHEMA, line_base)?;
    let source = FileText {
        file,
        text: yaml,
        line_base,
    };
    let mut diags = Diags::default();
    let empty = ("id is empty", "set id to a short place id");
    diags.extend(source.refuse_empty(&parsed.id, "id", Locator::Key("id:"), empty));
    diags.into_result(parsed)
}

fn compile_exits(file: &str, from: &str, exits: &[ExitYaml]) -> Result<Vec<Exit>, Vec<Diagnostic>> {
    let source = FileText::named(file);
    let mut diags = Diags::default();
    let mut compiled = Vec::new();
    let mut ids = BTreeSet::new();
    for (index, exit) in exits.iter().enumerate() {
        if exit.id.is_empty() || exit.to.is_empty() {
            diags.push(
                schema_error(file, "an exit needs an id and a to", "set both")
                    .at_path(format!("exits[{index}]")),
            );
            continue;
        }
        let path = format!("exits[{index}].id");
        let twice = ("exit", "is listed twice", "keep one exit with this id");
        let taken = !ids.insert(exit.id.clone());
        if let Some(diag) =
            taken.then(|| source.refuse_duplicate(&exit.id, &path, Locator::Nowhere, twice))
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
                ticks: 0,
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
            id: exit.id.clone(),
            // `name_exits` names an exit the pack leaves unnamed once every place has loaded.
            name: exit.name.clone().unwrap_or_default(),
            from: from.into(),
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
struct PlaceDoc {
    #[serde(rename = "schema")]
    _schema: String,
    id: String,
    name: Option<String>,
    facts: Vec<FactYaml>,
    #[serde(default)]
    exits: Vec<ExitYaml>,
    #[serde(default)]
    flags: Vec<String>,
    #[serde(default)]
    meters: Vec<MeterYaml>,
    #[serde(default)]
    rules: Vec<RuleYaml>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExitYaml {
    to: String,
    id: String,
    name: Option<String>,
    cost: Option<rule_list::CostYaml>,
    #[serde(default, rename = "if")]
    if_: Vec<rule_list::CondYaml>,
    #[serde(default)]
    locked_text: String,
    #[serde(default)]
    hidden: bool,
    revealed_by: Option<String>,
}
