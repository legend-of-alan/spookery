//! The modifier list in `rules/modifiers.yaml`: each modifier's meter and cap.
//! How long an application lasts is stated on the `modifier` effect that applies it.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::canon::refusal::{Diagnostics, FileText};
use crate::diagnostic::{parse_tagged, Diagnostic};
use crate::ecs::component::meter;
use crate::ecs::component::modifier;

pub(super) const FILE: &str = "rules/modifiers.yaml";
const SCHEMA: &str = "modifiers";

/// The modifiers file. Each modifier names one of the actor `meters`.
pub(super) fn load(
    text: &str,
    meters: &BTreeMap<String, meter::Def>,
) -> Result<BTreeMap<String, modifier::Def>, Vec<Diagnostic>> {
    let yaml: Yaml = parse_tagged(FILE, text, SCHEMA, false)?;
    let file_text = FileText { file: FILE, text };
    let mut diagnostics = Diagnostics::default();
    let mut modifiers = BTreeMap::new();
    for (index, entry) in yaml.modifiers.into_iter().enumerate() {
        let id = &entry.id;
        let max_stacks = usize::try_from(entry.max_stacks.unwrap_or(1)).unwrap_or(0);
        let constraints = [
            (
                !meters.contains_key(&entry.meter),
                "meter",
                format!(
                    "modifier '{id}' names meter '{}', which no actor carries",
                    entry.meter
                ),
                "name a meter in rules/meters.yaml or a cast file",
            ),
            (
                max_stacks == 0,
                "max_stacks",
                format!("modifier '{id}' has max_stacks below 1"),
                "set max_stacks to 1 or more, or leave it out for 1",
            ),
        ];
        let entry_diagnostics = file_text.refuse_entry(
            "modifier",
            index,
            id,
            modifiers.contains_key(id),
            constraints,
        );
        if entry_diagnostics.is_empty() {
            let def = modifier::Def {
                meter: entry.meter,
                max_stacks,
                hidden: entry.hidden,
            };
            modifiers.insert(entry.id, def);
        } else {
            diagnostics.extend(entry_diagnostics);
        }
    }
    diagnostics.into_result(modifiers)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Yaml {
    #[serde(rename = "schema")]
    _schema: String,
    modifiers: Vec<ModifierYaml>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModifierYaml {
    id: String,
    meter: String,
    max_stacks: Option<i64>,
    #[serde(default)]
    hidden: bool,
}
