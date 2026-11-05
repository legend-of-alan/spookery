//! The modifier list in `rules/modifiers.yaml`: each modifier's meter, ticks and cap.

use std::collections::BTreeMap;
use std::num::NonZeroU64;

use serde::Deserialize;

use crate::canon::source::{load_tagged, Diags, FileText, Locator, Problem, MODIFIERS_SCHEMA};
use crate::diagnostic::Diagnostic;
use crate::model::meter::MeterDef;
use crate::model::modifier::ModifierDef;

pub(super) const MODIFIERS_FILE: &str = "rules/modifiers.yaml";

/// The modifiers file. Each modifier names one of the actor `meters`.
pub(super) fn load_modifiers(
    text: &str,
    meters: &BTreeMap<String, MeterDef>,
) -> Result<BTreeMap<String, ModifierDef>, Vec<Diagnostic>> {
    let file: ModifiersFile = load_tagged(MODIFIERS_FILE, text, MODIFIERS_SCHEMA, 0)?;
    let source = FileText {
        file: MODIFIERS_FILE,
        text,
        line_base: 0,
    };
    let mut diags = Diags::default();
    let mut modifiers = BTreeMap::new();
    for (row, entry) in file.modifiers.into_iter().enumerate() {
        let path = format!("modifiers[{row}]");
        let refuse = |key: &'static str, message: String, hint: &'static str| {
            Problem {
                source: &source,
                path: &format!("{path}.{}", key.trim_end_matches(':')),
                at: Locator::Row {
                    list: "modifiers:",
                    row,
                    key,
                },
                message,
                hint,
            }
            .into_diagnostic()
        };
        let id = &entry.id;
        let max_stacks = usize::try_from(entry.max_stacks.unwrap_or(1)).unwrap_or(0);
        let checks = [
            (
                id.is_empty(),
                "id:",
                "a modifier id is empty".to_string(),
                "set id to a short name",
            ),
            (
                !meters.contains_key(&entry.meter),
                "meter:",
                format!(
                    "modifier '{id}' names meter '{}', which no actor carries",
                    entry.meter
                ),
                "name a meter in rules/meters.yaml or a cast file",
            ),
            (
                max_stacks == 0,
                "max_stacks:",
                format!("modifier '{id}' has max_stacks below 1"),
                "set max_stacks to 1 or more, or leave it out for 1",
            ),
            (
                modifiers.contains_key(id),
                "id:",
                format!("modifier '{id}' is listed twice"),
                "keep one modifier",
            ),
        ];
        let found: Vec<Diagnostic> = checks
            .into_iter()
            .filter(|(failed, ..)| *failed)
            .map(|(_, key, message, hint)| refuse(key, message, hint))
            .collect();
        if found.is_empty() {
            let def = ModifierDef {
                meter: entry.meter,
                ticks: entry.ticks.get(),
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
struct ModifiersFile {
    #[serde(rename = "schema")]
    _schema: String,
    modifiers: Vec<ModifierYaml>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModifierYaml {
    id: String,
    meter: String,
    /// The shape refuses 0, as it refuses any non-number.
    ticks: NonZeroU64,
    max_stacks: Option<i64>,
    #[serde(default)]
    hidden: bool,
}
