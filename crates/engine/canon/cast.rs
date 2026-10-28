//! `cast/*.md`: one cast member.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::canon::fact::{fact_ids_of, insert_fact, FactFile};
use crate::canon::rule_list::{self, RuleEntry};
use crate::canon::source::{load_headed, At, Diags, Page, Problem, Source, CAST_SCHEMA};
use crate::canon::{CastCanon, CastKind, FactCanon, TickProfile};
use crate::diagnostic::{schema as schema_error, Diagnostic};
use crate::rule::{Owner, Rule};

/// One cast page into `cast` and its facts into `facts`, and its rules. A bad fact stops
/// the file before its kind and rules.
pub(super) fn add_cast(
    page: Page,
    facts: &mut BTreeMap<String, FactCanon>,
    cast: &mut BTreeMap<String, CastCanon>,
    diags: &mut Diags,
) -> Option<Vec<Rule>> {
    let file = diags.keep(load_cast(&page.file, &page.yaml, 1))?;
    let mut fact_ok = true;
    for (index, fact) in file.facts.iter().enumerate() {
        let fact = FactFile {
            id: fact.id.clone(),
            text: fact.text.clone(),
            hidden: fact.hidden,
        };
        let inserted = insert_fact(facts, &page.file, &page.text, index, fact, &file.id);
        fact_ok &= diags.keep(inserted).is_some();
    }
    if !fact_ok {
        return None;
    }
    // `load_cast` validates `file.kind` is `"npc"` or `"pc_template"`.
    let kind = if file.kind == "npc" {
        CastKind::Npc
    } else {
        CastKind::PcTemplate
    };
    if cast.contains_key(&file.id) {
        diags.push(
            schema_error(
                &page.file,
                &format!("cast id '{}' is already defined", file.id),
                "keep one cast file with this id",
            )
            .with_path("id"),
        );
        return None;
    }
    let id = file.id.clone();
    cast.insert(
        id.clone(),
        CastCanon {
            kind,
            display_name: file.display_name,
            facts: fact_ids_of(facts, &id),
            body: page.body,
            tick: file.tick,
        },
    );
    diags.keep(rule_list::compile(&page.file, file.rules, &Owner::Cast(id)))
}

fn load_cast(file: &str, yaml: &str, line_base: u32) -> Result<CastFile, Vec<Diagnostic>> {
    let parsed: CastFile = load_headed(file, yaml, CAST_SCHEMA, line_base)?;
    let source = Source {
        file,
        text: yaml,
        line_base,
    };
    let mut diags = Diags::default();
    if parsed.id.is_empty() {
        diags.push(
            Problem {
                source: &source,
                path: "id",
                at: At::Key("id:"),
                problem: "id is empty".into(),
                hint: "set id to a short cast id",
            }
            .schema(),
        );
    }
    if parsed.id == crate::effect::WORLD {
        diags.push(
            Problem {
                source: &source,
                path: "id",
                at: At::Key("id:"),
                problem: "id world is reserved".into(),
                hint: "rename this cast member",
            }
            .schema(),
        );
    }
    if parsed.display_name.trim().is_empty() {
        diags.push(
            Problem {
                source: &source,
                path: "display_name",
                at: At::Key("display_name:"),
                problem: "display_name is empty".into(),
                hint: "set display_name to the name players see",
            }
            .schema(),
        );
    }
    if parsed.kind != "npc" && parsed.kind != "pc_template" {
        diags.push(
            Problem {
                source: &source,
                path: "kind",
                at: At::Key("kind:"),
                problem: format!("kind is '{}', which is not npc or pc_template", parsed.kind),
                hint: "set kind to npc or pc_template",
            }
            .schema(),
        );
    }
    diags.result(parsed)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CastFile {
    #[serde(rename = "schema")]
    _schema: String,
    id: String,
    kind: String,
    display_name: String,
    facts: Vec<FactFile>,
    #[serde(default)]
    tick: Option<TickProfile>,
    #[serde(default)]
    rules: Vec<RuleEntry>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canon::tests::rejects;

    #[test]
    fn load_cast_rejects_an_unknown_tag_and_the_old_versioned_tag() {
        rejects(
            |text| load_cast("cast/mira.md", text, 0).map(|_| ()),
            "cast",
            concat!("cast/", "v0"),
            "id: mira\nkind: npc\ndisplay_name: Mira\nfacts: []\n",
        );
    }
}
