//! `cast/*.md`: one cast member.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::canon::meters::{self, meter_descs, MeterYaml};
use crate::canon::pack_files::Page;
use crate::canon::refusal::{Diags, FileText, Locator};
use crate::canon::rule_list::{self, RuleYaml};
use crate::canon::{self, CastDef, IdleProfile};
use crate::diagnostic::{parse_tagged, Diagnostic};
use crate::ecs::component::meter;
use crate::ecs::id::ActorId;
use crate::entity;
use crate::rules::rule::{Owner, Rule};

const CAST_SCHEMA: &str = "cast";

/// One cast page into `cast`, its own meters into `meters` when those loaded, and its rules.
pub(super) fn add(
    page: Page,
    cast: &mut BTreeMap<ActorId, CastDef>,
    meters: Option<&mut BTreeMap<String, meter::Descriptor>>,
    diags: &mut Diags,
) -> Option<Vec<Rule>> {
    let doc = diags.keep(load(&page.file_path, &page.yaml))?;
    let whole = FileText {
        file_path: &page.file_path,
        text: &page.text,
    };
    let control_type = if doc.control_type == "npc" {
        canon::ControlType::Npc
    } else {
        canon::ControlType::PcTemplate
    };
    let hint = "keep one cast file with this id";
    let twice = ("cast id", "is already defined", hint);
    let taken = cast.contains_key(doc.id.as_str());
    if let Some(diag) =
        taken.then(|| whole.refuse_duplicate(&doc.id, "id", Locator::Nowhere, twice))
    {
        diags.push(diag);
        return None;
    }
    let id: ActorId = doc.id.clone().into();
    let carrier = Some(entity::Id::Actor(id.clone()));
    let own = diags.keep(meter_descs(&whole, doc.meters, carrier));
    if let (Some(meters), Some(own)) = (meters, own) {
        meters::add_own(meters, own, None, &whole, diags);
    }
    cast.insert(
        id.clone(),
        CastDef {
            control_type,
            name: doc.name.unwrap_or(doc.id),
            body: page.body,
            idle: doc.idle,
        },
    );
    diags.keep(rule_list::rules_from_yaml(
        &page.file_path,
        doc.rules,
        &Owner::Cast(id.into()),
    ))
}

fn load(file_path: &str, yaml: &str) -> Result<CastDoc, Vec<Diagnostic>> {
    let parsed: CastDoc = parse_tagged(file_path, yaml, CAST_SCHEMA, true)?;
    let source = FileText {
        file_path,
        text: yaml,
    };
    let mut diags = Diags::default();
    let empty = ("id is empty", "set id to a short cast id");
    diags.extend(source.refuse_empty(&parsed.id, "id", Locator::Key("id:"), empty));
    if parsed.id == "world" {
        let message = "id world is reserved".into();
        diags.push(source.refuse_at(
            "id",
            Locator::Key("id:"),
            message,
            "rename this cast member",
        ));
    }
    if parsed.control_type != "npc" && parsed.control_type != "pc_template" {
        let message = format!(
            "control_type is '{}', which is not npc or pc_template",
            parsed.control_type
        );
        let at = Locator::Key("control_type:");
        diags.push(source.refuse_at(
            "control_type",
            at,
            message,
            "set control_type to npc or pc_template",
        ));
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
