//! `cast/*.md`: one cast member.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::canon::meters::{add_own_meters, meter_defs, MeterYaml};
use crate::canon::rule_list::{self, RuleYaml};
use crate::canon::source::{load_tagged, Diags, FileText, Locator, Page, Problem, CAST_SCHEMA};
use crate::canon::{self, CastDef, IdleProfile};
use crate::diagnostic::Diagnostic;
use crate::entity;
use crate::rules::rule::{Owner, Rule};
use crate::state::component::meter::MeterDef;
use crate::state::id::ActorId;

/// One cast page into `cast`, its own meters into `meters` when those loaded, and its rules.
pub(super) fn add_cast(
    page: Page,
    cast: &mut BTreeMap<ActorId, CastDef>,
    meters: Option<&mut BTreeMap<String, MeterDef>>,
    diags: &mut Diags,
) -> Option<Vec<Rule>> {
    let file = diags.keep(load_cast(&page.file, &page.yaml, 1))?;
    let whole = FileText {
        file: &page.file,
        text: &page.text,
        line_base: 0,
    };
    // `load_cast` validates `file.control_type` is `"npc"` or `"pc_template"`.
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
    let own = diags.keep(meter_defs(&whole, file.meters, carrier));
    if let (Some(meters), Some(own)) = (meters, own) {
        add_own_meters(meters, own, None, &whole, diags);
    }
    cast.insert(
        id.clone(),
        CastDef {
            control_type,
            display_name: file.display_name,
            flags: file.flags,
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

fn load_cast(file: &str, yaml: &str, line_base: u32) -> Result<CastDoc, Vec<Diagnostic>> {
    let parsed: CastDoc = load_tagged(file, yaml, CAST_SCHEMA, line_base)?;
    let source = FileText {
        file,
        text: yaml,
        line_base,
    };
    let mut diags = Diags::default();
    let empty = ("id is empty", "set id to a short cast id");
    diags.extend(source.refuse_empty(&parsed.id, "id", Locator::Key("id:"), empty));
    // `of: world` names the world, so no cast member can take that id.
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
    diags.extend(source.refuse_empty_display_name(&parsed.display_name));
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
    #[serde(rename = "schema")]
    _schema: String,
    id: String,
    control_type: String,
    display_name: String,
    #[serde(default)]
    flags: Vec<String>,
    /// The meters only this actor carries.
    #[serde(default)]
    meters: Vec<MeterYaml>,
    idle: Option<IdleProfile>,
    #[serde(default)]
    rules: Vec<RuleYaml>,
}
