//! `cast/*.md`: one cast member.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::canon::fact::{FactFile, Facts};
use crate::canon::rule_list::{self, RuleEntry};
use crate::canon::source::{load_headed, At, Diags, Page, Problem, Source, CAST_SCHEMA};
use crate::canon::{CastCanon, CastKind, TickProfile};
use crate::diagnostic::Diagnostic;
use crate::rule::{Owner, Rule};

/// One cast page into `cast` and its facts into `facts`, and its rules. A bad fact stops
/// the file before its kind and rules.
pub(super) fn add_cast(
    page: Page,
    facts: &mut Facts,
    cast: &mut BTreeMap<String, CastCanon>,
    diags: &mut Diags,
) -> Option<Vec<Rule>> {
    let file = diags.keep(load_cast(&page.file, &page.yaml, 1))?;
    let whole = Source {
        file: &page.file,
        text: &page.text,
        line_base: 0,
    };
    let ids = diags.keep(facts.add(&whole, file.facts, &file.id))?;
    // `load_cast` validates `file.kind` is `"npc"` or `"pc_template"`.
    let kind = if file.kind == "npc" {
        CastKind::Npc
    } else {
        CastKind::PcTemplate
    };
    let hint = "keep one cast file with this id";
    let twice = ("cast id", "is already defined", hint);
    let taken = cast.contains_key(&file.id);
    if let Some(diag) = taken.then(|| whole.repeat(&file.id, "id", At::Nowhere, twice)) {
        diags.push(diag);
        return None;
    }
    let id = file.id.clone();
    cast.insert(
        id.clone(),
        CastCanon {
            kind,
            display_name: file.display_name,
            facts: ids,
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
    let empty = ("id is empty", "set id to a short cast id");
    diags.extend(source.required(&parsed.id, "id", At::Key("id:"), empty));
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
    diags.extend(source.display_name(&parsed.display_name));
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
