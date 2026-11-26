//! `cast/*.md`: one cast member.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::canon;
use crate::canon::meters::{self, MeterYaml};
use crate::canon::pack_files::Page;
use crate::canon::refusal::{Diags, FileText, Locator, Problem};
use crate::canon::rule_list::{self, parse::RuleYaml};
use crate::diagnostic::{parse_tagged, Diagnostic};
use crate::ecs::component::meter;
use crate::ecs::id::ActorId;
use crate::entity;
use crate::rules::rule::{Owner, Rule};

const SCHEMA: &str = "cast";

#[derive(Clone, Debug)]
pub struct Def {
    pub control_type: canon::ControlType,
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
    meters: Option<&mut BTreeMap<String, meter::Descriptor>>,
    diags: &mut Diags,
) -> Option<Vec<Rule>> {
    let file = diags.keep(parse(&page.file, &page.yaml))?;
    let whole = FileText {
        file: &page.file,
        text: &page.text,
    };
    let control_type = if file.control_type == "npc" {
        canon::ControlType::Npc
    } else {
        canon::ControlType::PcTemplate
    };
    let hint = "keep one cast file with this id";
    let twice = ("cast id", "is already defined", hint);
    let taken = cast.contains_key(file.id.as_str());
    if let Some(diag) =
        taken.then(|| whole.refuse_duplicate(&file.id, "id", Locator::Nowhere, twice))
    {
        diags.push(diag);
        return None;
    }
    let id: ActorId = file.id.clone().into();
    let carrier = Some(entity::Id::Actor(id.clone()));
    let own = diags.keep(meters::compile(&whole, file.meters, carrier));
    if let (Some(meters), Some(own)) = (meters, own) {
        meters::add_own(meters, own, None, &whole, diags);
    }
    cast.insert(
        id.clone(),
        Def {
            control_type,
            name: file.name.unwrap_or(file.id),
            body: page.body,
            idle: file.idle,
        },
    );
    diags.keep(rule_list::compile(
        &page.file,
        file.rules,
        &Owner::Cast(id.into()),
    ))
}

fn parse(file: &str, yaml: &str) -> Result<CastDoc, Vec<Diagnostic>> {
    let parsed: CastDoc = parse_tagged(file, yaml, SCHEMA, true)?;
    let source = FileText { file, text: yaml };
    let mut diags = Diags::default();
    let empty = ("id is empty", "set id to a short cast id");
    diags.extend(source.refuse_empty(&parsed.id, "id", Locator::Key("id:"), empty));
    if parsed.id == "world" {
        diags.push(
            Problem {
                source: &source,
                path: "id",
                at: Locator::Key("id:"),
                message: "id world is reserved".into(),
                hint: "rename this cast member",
            }
            .into_diagnostic(),
        );
    }
    if parsed.control_type != "npc" && parsed.control_type != "pc_template" {
        diags.push(
            Problem {
                source: &source,
                path: "control_type",
                at: Locator::Key("control_type:"),
                message: format!(
                    "control_type is '{}', which is not npc or pc_template",
                    parsed.control_type
                ),
                hint: "set control_type to npc or pc_template",
            }
            .into_diagnostic(),
        );
    }
    diags.into_result(parsed)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CastDoc {
    // repeats module: every file shape in canon ends in Doc
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
