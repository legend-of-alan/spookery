//! The modifier list in `rules/modifiers.yaml`: each modifier's meter and cap.
//! How long an application lasts is stated on the `modifier` effect that applies it.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::canon::refusal::{Diags, FileText};
use crate::diagnostic::{parse_tagged, Diagnostic};
use crate::ecs::component::meter;
use crate::ecs::component::modifier;

const MODIFIERS_SCHEMA: &str = "modifiers";

pub(super) const MODIFIERS_FILE: &str = "rules/modifiers.yaml";

/// The modifiers file. Each modifier names one of the actor `meters`.
pub(super) fn load(
    text: &str,
    meters: &BTreeMap<String, meter::Descriptor>,
) -> Result<BTreeMap<String, modifier::Descriptor>, Vec<Diagnostic>> {
    let doc: ModifiersDoc = parse_tagged(MODIFIERS_FILE, text, MODIFIERS_SCHEMA, false)?;
    let source = FileText {
        file_path: MODIFIERS_FILE,
        text,
    };
    let mut diags = Diags::default();
    let mut modifiers = BTreeMap::new();
    for (row, entry) in doc.modifiers.into_iter().enumerate() {
        let id = &entry.id;
        let max_stacks = usize::try_from(entry.max_stacks.unwrap_or(1)).unwrap_or(0);
        let rules = [
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
        let found = source.refuse_row("modifier", row, id, modifiers.contains_key(id), rules);
        if found.is_empty() {
            let desc = modifier::Descriptor {
                meter: entry.meter,
                max_stacks,
                hidden: entry.hidden,
            };
            modifiers.insert(entry.id, desc);
        } else {
            diags.extend(found);
        }
    }
    diags.into_result(modifiers)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModifiersDoc {
    // repeats module: every file shape in canon ends in Doc
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
