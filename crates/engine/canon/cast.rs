//! `cast/*.md`: one cast member.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::canon::meters::{self, MeterYaml};
use crate::canon::pack_files::Page;
use crate::canon::refusal::{Diagnostics, FileText, Locator, Problem};
use crate::canon::rule_list::{self, parse::RuleYaml};
use crate::diagnostic::{parse_tagged, Diagnostic};
use crate::ecs::component::meter;
use crate::ecs::id::ActorId;
use crate::entity;
use crate::rules::rule::{Owner, Rule};

const SCHEMA: &str = "cast";

#[derive(Clone, Debug)]
pub struct Def {
    pub control_type: spookery_protocol::ControlType,
    /// The name a player reads, the id when the pack gives none.
    pub name: String,
    pub body: String,
    pub idle: Option<IdleProfile>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdleProfile {
    pub eligible: bool,
}

/// One cast page into `cast`, its own meters into `meters` when those loaded, and its rules.
pub(super) fn add(
    page: Page,
    cast: &mut BTreeMap<ActorId, Def>,
    meters: Option<&mut BTreeMap<String, meter::Def>>,
    diagnostics: &mut Diagnostics,
) -> Option<Vec<Rule>> {
    let yaml = diagnostics.keep(parse(&page.file, &page.frontmatter))?;
    let page_text = FileText {
        file: &page.file,
        text: &page.text,
    };
    let control_type = if yaml.control_type == "npc" {
        spookery_protocol::ControlType::Npc
    } else {
        spookery_protocol::ControlType::PcTemplate
    };
    let hint = "keep one cast file with this id";
    let duplicate_words = ("cast id", "is already defined", hint);
    let is_duplicate = cast.contains_key(yaml.id.as_str());
    if let Some(diagnostic) = is_duplicate
        .then(|| page_text.refuse_duplicate(&yaml.id, "id", Locator::Nowhere, duplicate_words))
    {
        diagnostics.push(diagnostic);
        return None;
    }
    let id: ActorId = yaml.id.clone().into();
    let entity_id = Some(entity::Id::Actor(id.clone()));
    let own = diagnostics.keep(meters::compile(&page_text, yaml.meters, entity_id));
    if let (Some(meters), Some(own)) = (meters, own) {
        meters::add_own(meters, own, None, &page_text, diagnostics);
    }
    cast.insert(
        id.clone(),
        Def {
            control_type,
            name: yaml.name.unwrap_or(yaml.id),
            body: page.body,
            idle: yaml.idle,
        },
    );
    diagnostics.keep(rule_list::compile(
        &page.file,
        yaml.rules,
        &Owner::Cast(id.into()),
    ))
}

fn parse(file: &str, frontmatter: &str) -> Result<Yaml, Vec<Diagnostic>> {
    let yaml: Yaml = parse_tagged(file, frontmatter, SCHEMA, true)?;
    let file_text = FileText {
        file,
        text: frontmatter,
    };
    let mut diagnostics = Diagnostics::default();
    let empty_words = ("id is empty", "set id to a short cast id");
    diagnostics.extend(file_text.refuse_empty(&yaml.id, "id", Locator::Key("id:"), empty_words));
    if yaml.id == "world" {
        diagnostics.push(
            Problem {
                source: &file_text,
                path: "id",
                at: Locator::Key("id:"),
                message: "id world is reserved".into(),
                hint: "rename this cast member",
            }
            .into_diagnostic(),
        );
    }
    if yaml.control_type != "npc" && yaml.control_type != "pc_template" {
        diagnostics.push(
            Problem {
                source: &file_text,
                path: "control_type",
                at: Locator::Key("control_type:"),
                message: format!(
                    "control_type is '{}', which is not npc or pc_template",
                    yaml.control_type
                ),
                hint: "set control_type to npc or pc_template",
            }
            .into_diagnostic(),
        );
    }
    diagnostics.into_result(yaml)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Yaml {
    #[serde(rename = "schema")]
    _schema: String,
    id: String,
    control_type: String,
    name: Option<String>,
    /// The meters only this actor carries.
    #[serde(default)]
    meters: Vec<MeterYaml>,
    idle: Option<IdleProfile>,
    #[serde(default)]
    rules: Vec<RuleYaml>,
}
