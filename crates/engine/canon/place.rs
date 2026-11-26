//! `world/places/*.md`: a place, and the meters it carries.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::canon::meters::{self, MeterYaml};
use crate::canon::pack_files::Page;
use crate::canon::refusal::{Diags, FileText, Locator};
use crate::canon::rule_list::{self, parse::RuleYaml};
use crate::diagnostic::{parse_tagged, Diagnostic};
use crate::ecs::component::meter;
use crate::ecs::id::PlaceId;
use crate::entity;
use crate::rules::rule::{Owner, Rule};

const SCHEMA: &str = "place";

#[derive(Clone, Debug)]
pub struct Def {
    /// The name a player reads, the id when the pack gives none.
    pub name: String,
    pub body: String,
}

/// The meters one place file declares, by that file, for `load_canon` to add once the meters
/// every actor carries have loaded.
pub(super) type Meters = Vec<(String, BTreeMap<String, meter::Descriptor>)>;

/// One place page into `places` and its meters into `meters`, and its rules.
pub(super) fn add(
    page: Page,
    places: &mut BTreeMap<PlaceId, Def>,
    meters: &mut Meters,
    diags: &mut Diags,
) -> Option<Vec<Rule>> {
    let file = diags.keep(parse(&page.file, &page.yaml))?;
    let whole = FileText {
        file: &page.file,
        text: &page.text,
    };
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
    if let Some(own) = diags.keep(meters::compile(&whole, file.meters, carrier)) {
        meters.push((page.file.clone(), own));
    }
    places.insert(
        id,
        Def {
            name: file.name.unwrap_or_else(|| file.id.clone()),
            body: page.body,
        },
    );
    diags.keep(rule_list::compile(
        &page.file,
        file.rules,
        &Owner::Place(file.id),
    ))
}

fn parse(file: &str, yaml: &str) -> Result<PlaceDoc, Vec<Diagnostic>> {
    let parsed: PlaceDoc = parse_tagged(file, yaml, SCHEMA, true)?;
    let source = FileText { file, text: yaml };
    let mut diags = Diags::default();
    let empty = ("id is empty", "set id to a short place id");
    diags.extend(source.refuse_empty(&parsed.id, "id", Locator::Key("id:"), empty));
    diags.into_result(parsed)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlaceDoc {
    // repeats module: every file shape in canon ends in Doc
    #[serde(rename = "schema")]
    _schema: String,
    id: String,
    name: Option<String>,
    #[serde(default)]
    meters: Vec<MeterYaml>,
    #[serde(default)]
    rules: Vec<RuleYaml>,
}
