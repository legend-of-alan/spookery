//! Meter lists: `rules/meters.yaml` for every actor, `world/meters.yaml` for the world, and
//! the `meters:` of a cast, place or story file for that one entity.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::canon::refusal::{Diagnostics, DuplicateWords, FileText, Locator};
use crate::diagnostic::{parse_tagged, Diagnostic};
use crate::ecs::component::meter;
use crate::entity;

const SCHEMA: &str = "meters";

/// A meters file, whose meters `entity_id` carries.
pub(super) fn load(
    file: &str,
    text: &str,
    entity_id: Option<entity::Id>,
) -> Result<BTreeMap<String, meter::Def>, Vec<Diagnostic>> {
    let yaml: Yaml = parse_tagged(file, text, SCHEMA, false)?;
    let file_text = FileText { file, text };
    compile(&file_text, yaml.meters, entity_id)
}

/// The meter entries of one file. A meter no actor carries is hidden unless its entry says
/// `hidden: false`.
pub(super) fn compile(
    file_text: &FileText,
    entries: Vec<MeterYaml>,
    entity_id: Option<entity::Id>,
) -> Result<BTreeMap<String, meter::Def>, Vec<Diagnostic>> {
    let hidden_by_default = !matches!(entity_id, None | Some(entity::Id::Actor(_)));
    let mut diagnostics = Diagnostics::default();
    let mut meters = BTreeMap::new();
    for (index, entry) in entries.into_iter().enumerate() {
        let id = &entry.id;
        let constraints = [
            (
                entry.min > entry.base_cap,
                "min",
                format!("meter '{id}' has min above base_cap"),
                "set min at or below base_cap",
            ),
            (
                entry.base_cap > entry.max,
                "base_cap",
                format!("meter '{id}' has base_cap above max"),
                "set base_cap at or below max",
            ),
            (
                !(entry.min..=entry.base_cap).contains(&entry.base),
                "base",
                format!("meter '{id}' has base outside min and base_cap"),
                "set base between min and base_cap",
            ),
        ];
        let entry_diagnostics =
            file_text.refuse_entry("meter", index, id, meters.contains_key(id), constraints);
        if !entry_diagnostics.is_empty() {
            diagnostics.extend(entry_diagnostics);
            continue;
        }
        meters.insert(
            entry.id,
            meter::Def {
                name: entry.name.unwrap_or_default(),
                min: entry.min,
                base_cap: entry.base_cap,
                max: entry.max,
                base: entry.base,
                hidden: entry.hidden.unwrap_or(hidden_by_default),
                entity_id: entity_id.clone(),
            },
        );
    }
    diagnostics.into_result(meters)
}

/// Add the meters one cast, place or story file declares to `meters`. An id that
/// `rules/meters.yaml`, another file or, when given, `world_meters` has already is refused.
pub(super) fn add_own(
    meters: &mut BTreeMap<String, meter::Def>,
    own: BTreeMap<String, meter::Def>,
    world_meters: Option<&BTreeMap<String, meter::Def>>,
    file_text: &FileText,
    diagnostics: &mut Diagnostics,
) {
    const KEEP_ONE: &str = "keep one meter with this id";
    for (key, def) in own {
        let duplicate_entity_id =
            if world_meters.is_some_and(|world_meters| world_meters.contains_key(&key)) {
                Some(Some(entity::Id::Global))
            } else {
                meters
                    .get(&key)
                    .map(|earlier_def| earlier_def.entity_id.clone())
            };
        let (after, hint) = match duplicate_entity_id {
            None => {
                meters.insert(key, def);
                continue;
            }
            Some(None) => (
                "is already in rules/meters.yaml",
                "rename it, or drop it and use the one every actor carries",
            ),
            Some(Some(entity::Id::Global)) => ("is already in world/meters.yaml", KEEP_ONE),
            Some(Some(_)) => (
                "is already declared in another cast, place or story file",
                KEEP_ONE,
            ),
        };
        let duplicate_words: DuplicateWords = ("meter", after, hint);
        let key_path = format!("meters.{key}");
        diagnostics.push(file_text.refuse_duplicate(
            &key,
            &key_path,
            Locator::Nowhere,
            duplicate_words,
        ));
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Yaml {
    #[serde(rename = "schema")]
    _schema: String,
    meters: Vec<MeterYaml>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MeterYaml {
    id: String,
    name: Option<String>,
    min: i64,
    base_cap: i64,
    max: i64,
    base: i64,
    hidden: Option<bool>,
}

#[cfg(test)]
mod tests {
    use crate::diagnostic::first_reason;
    use crate::Reason;

    #[test]
    fn load_meters_rejects_an_unknown_key_at_its_line() {
        let unknown = "schema: meters
meters:
  - id: vigor
    min: 0
    base_cap: 1
    max: 1
    base: 1
    kind: stat
";
        let err = super::load("rules/meters.yaml", unknown, None).unwrap_err();
        assert_eq!(err.len(), 1, "{err:?}");
        assert!(err[0].message.contains("unknown field 'kind'"), "{err:?}");
        assert_eq!(err[0].line, Some(8), "{err:?}");
        assert_eq!(first_reason(&err), Reason::Schema);
    }
}
