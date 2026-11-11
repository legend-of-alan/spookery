//! The modifier list in `rules/modifiers.yaml`: each modifier's meter and cap.
//! How long an application lasts is stated on the `modifier` effect that applies it.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::canon::source::{load_tagged, Diags, FileText, MODIFIERS_SCHEMA};
use crate::diagnostic::Diagnostic;
use crate::state::component::meter::MeterDef;
use crate::state::component::modifier::ModifierDef;

pub(super) const MODIFIERS_FILE: &str = "rules/modifiers.yaml";

/// The modifiers file. Each modifier names one of the actor `meters`.
pub(super) fn load_modifiers(
    text: &str,
    meters: &BTreeMap<String, MeterDef>,
) -> Result<BTreeMap<String, ModifierDef>, Vec<Diagnostic>> {
    let doc: ModifiersDoc = load_tagged(MODIFIERS_FILE, text, MODIFIERS_SCHEMA, 0)?;
    let source = FileText {
        file: MODIFIERS_FILE,
        text,
        line_base: 0,
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
            let def = ModifierDef {
                meter: entry.meter,
                max_stacks,
                hidden: entry.hidden,
            };
            modifiers.insert(entry.id, def);
        } else {
            diags.extend(found);
        }
    }
    diags.into_result(modifiers)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModifiersDoc {
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
