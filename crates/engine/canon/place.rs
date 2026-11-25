//! `world/places/*.md`: a place, and the meters it carries.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::canon::meters::{meter_descs, MeterYaml};
use crate::canon::pack_files::Page;
use crate::canon::refusal::{Diags, FileText, Locator};
use crate::canon::rule_list::{self, RuleYaml};
use crate::canon::PlaceDef;
use crate::diagnostic::{parse_tagged, Diagnostic};
use crate::ecs::component::meter;
use crate::ecs::id::PlaceId;
use crate::entity;
use crate::rules::rule::{Owner, Rule};

const PLACE_SCHEMA: &str = "place";

/// The meters one place file declares, by that file, for `load_canon` to add once the meters
/// every actor carries have loaded.
pub(super) type Meters = Vec<(String, BTreeMap<String, meter::Descriptor>)>;

/// One place page into `places` and its meters into `meters`, and its rules.
pub(super) fn add(
    page: Page,
    places: &mut BTreeMap<PlaceId, PlaceDef>,
    meters: &mut Meters,
    diags: &mut Diags,
) -> Option<Vec<Rule>> {
    let doc = diags.keep(load(&page.file_path, &page.yaml))?;
    let whole = FileText {
        file_path: &page.file_path,
        text: &page.text,
    };
    let twice = ("place", "is already defined", "keep one place with this id");
    let taken = places.contains_key(doc.id.as_str());
    if let Some(diag) =
        taken.then(|| whole.refuse_duplicate(&doc.id, "id", Locator::Nowhere, twice))
    {
        diags.push(diag);
        return None;
    }
    let id: PlaceId = doc.id.clone().into();
    let carrier = Some(entity::Id::Place(id.clone()));
    if let Some(own) = diags.keep(meter_descs(&whole, doc.meters, carrier)) {
        meters.push((page.file_path.clone(), own));
    }
    places.insert(
        id,
        PlaceDef {
            name: doc.name.unwrap_or_else(|| doc.id.clone()),
            body: page.body,
        },
    );
    diags.keep(rule_list::rules_from_yaml(
        &page.file_path,
        doc.rules,
        &Owner::Place(doc.id),
    ))
}

fn load(file_path: &str, yaml: &str) -> Result<PlaceDoc, Vec<Diagnostic>> {
    let parsed: PlaceDoc = parse_tagged(file_path, yaml, PLACE_SCHEMA, true)?;
    let source = FileText {
        file_path,
        text: yaml,
    };
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
