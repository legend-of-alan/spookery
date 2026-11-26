//! `world/places/*.md`: a place, and the meters it carries.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::canon::meters::{self, MeterYaml};
use crate::canon::pack_files::Page;
use crate::canon::refusal::{Diagnostics, FileText, Locator};
use crate::canon::rule_list::{self, parse::RuleYaml};
use crate::diagnostic::{parse_tagged, Diagnostic};
use crate::ecs::component::meter;
use crate::ecs::id::PlaceId;
use crate::entity;
use crate::rules::rule::{Rule, Scope};

const SCHEMA: &str = "place";

#[derive(Clone, Debug)]
pub struct Def {
    /// The name a player reads, the id when the pack gives none.
    pub name: String,
    pub body: String,
}

/// The meters one place file declares, by that file, for `load_canon` to add once the meters
/// every actor carries have loaded.
pub(super) type MetersByFile = Vec<(String, BTreeMap<String, meter::Def>)>;

/// One place page into `places` and its meters into `meters`, and its rules.
pub(super) fn add(
    page: Page,
    places: &mut BTreeMap<PlaceId, Def>,
    meters: &mut MetersByFile,
    diagnostics: &mut Diagnostics,
) -> Option<Vec<Rule>> {
    let yaml = diagnostics.keep(parse(&page.file, &page.frontmatter))?;
    let page_text = FileText {
        file: &page.file,
        text: &page.text,
    };
    let duplicate_words = ("place", "is already defined", "keep one place with this id");
    let is_duplicate = places.contains_key(yaml.id.as_str());
    if let Some(diagnostic) = is_duplicate
        .then(|| page_text.refuse_duplicate(&yaml.id, "id", Locator::Nowhere, duplicate_words))
    {
        diagnostics.push(diagnostic);
        return None;
    }
    let id: PlaceId = yaml.id.clone().into();
    let entity_id = Some(entity::Id::Place(id.clone()));
    if let Some(own) = diagnostics.keep(meters::compile(&page_text, yaml.meters, entity_id)) {
        meters.push((page.file.clone(), own));
    }
    places.insert(
        id,
        Def {
            name: yaml.name.unwrap_or_else(|| yaml.id.clone()),
            body: page.body,
        },
    );
    diagnostics.keep(rule_list::compile(
        &page.file,
        yaml.rules,
        &Scope::Place(yaml.id),
    ))
}

fn parse(file: &str, frontmatter: &str) -> Result<Yaml, Vec<Diagnostic>> {
    let yaml: Yaml = parse_tagged(file, frontmatter, SCHEMA, true)?;
    let file_text = FileText {
        file,
        text: frontmatter,
    };
    let mut diagnostics = Diagnostics::default();
    let empty_words = ("id is empty", "set id to a short place id");
    diagnostics.extend(file_text.refuse_empty(&yaml.id, "id", Locator::Key("id:"), empty_words));
    diagnostics.into_result(yaml)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Yaml {
    #[serde(rename = "schema")]
    _schema: String,
    id: String,
    name: Option<String>,
    #[serde(default)]
    meters: Vec<MeterYaml>,
    #[serde(default)]
    rules: Vec<RuleYaml>,
}
