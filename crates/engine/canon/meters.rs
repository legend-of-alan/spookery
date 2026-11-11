//! Meter lists: `rules/meters.yaml` for every actor, `world/meters.yaml` for the world, and
//! the `meters:` of a cast, place or story file for that one entity.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::canon::source::{load_tagged, Diags, DuplicateWords, FileText, Locator, METERS_SCHEMA};
use crate::diagnostic::Diagnostic;
use crate::entity;
use crate::state::component::meter::MeterDef;

/// A meters file, whose meters `carrier` carries.
pub(super) fn load_meters(
    rel: &str,
    text: &str,
    carrier: Option<entity::Id>,
) -> Result<BTreeMap<String, MeterDef>, Vec<Diagnostic>> {
    let doc: MetersDoc = load_tagged(rel, text, METERS_SCHEMA, 0)?;
    let source = FileText {
        file: rel,
        text,
        line_base: 0,
    };
    meter_defs(&source, doc.meters, carrier)
}

/// The meter rows of one file. A meter no actor carries is hidden unless its row says
/// `hidden: false`.
pub(super) fn meter_defs(
    source: &FileText,
    rows: Vec<MeterYaml>,
    carrier: Option<entity::Id>,
) -> Result<BTreeMap<String, MeterDef>, Vec<Diagnostic>> {
    let hidden_by_default = !matches!(carrier, None | Some(entity::Id::Actor(_)));
    let mut diags = Diags::default();
    let mut meters = BTreeMap::new();
    for (row, entry) in rows.into_iter().enumerate() {
        let id = &entry.id;
        let rules = [
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
        let found = source.refuse_row("meter", row, id, meters.contains_key(id), rules);
        if !found.is_empty() {
            diags.extend(found);
            continue;
        }
        meters.insert(
            entry.id,
            MeterDef {
                min: entry.min,
                base_cap: entry.base_cap,
                max: entry.max,
                start: entry.base,
                hidden: entry.hidden.unwrap_or(hidden_by_default),
                carrier: carrier.clone(),
            },
        );
    }
    diags.into_result(meters)
}

/// Add the meters one cast, place or story file declares to `meters`. An id that
/// `rules/meters.yaml`, another file or, when given, `world` has already is refused.
pub(super) fn add_own_meters(
    meters: &mut BTreeMap<String, MeterDef>,
    own: BTreeMap<String, MeterDef>,
    world: Option<&BTreeMap<String, MeterDef>>,
    source: &FileText,
    diags: &mut Diags,
) {
    const KEEP_ONE: &str = "keep one meter with this id";
    for (key, def) in own {
        let taken = if world.is_some_and(|world| world.contains_key(&key)) {
            Some(Some(entity::Id::Global))
        } else {
            meters.get(&key).map(|taken| taken.carrier.clone())
        };
        let (after, hint) = match taken {
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
        let twice: DuplicateWords = ("meter", after, hint);
        let path = format!("meters.{key}");
        diags.push(source.refuse_duplicate(&key, &path, Locator::Nowhere, twice));
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MetersDoc {
    #[serde(rename = "schema")]
    _schema: String,
    meters: Vec<MeterYaml>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MeterYaml {
    id: String,
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
  - id: grit
    min: 0
    base_cap: 1
    max: 1
    base: 1
    kind: stat
";
        let err = super::load_meters("rules/meters.yaml", unknown, None).unwrap_err();
        assert_eq!(err.len(), 1, "{err:?}");
        assert!(err[0].message.contains("unknown field 'kind'"), "{err:?}");
        assert_eq!(err[0].line, Some(8), "{err:?}");
        assert_eq!(first_reason(&err), Reason::Schema);
    }
}
