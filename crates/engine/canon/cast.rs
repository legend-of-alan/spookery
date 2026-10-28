//! `cast/*.md`: one cast member.

use serde::Deserialize;

use crate::canon::fact::FactFile;
use crate::canon::rule_list::RuleEntry;
use crate::canon::source::{load_headed, At, Problem, Source, CAST_SCHEMA};
use crate::canon::TickProfile;
use crate::diagnostic::Diagnostic;

pub(super) fn load_cast(
    file: &str,
    yaml: &str,
    line_base: u32,
) -> Result<CastFile, Vec<Diagnostic>> {
    let parsed: CastFile = load_headed(file, yaml, CAST_SCHEMA, line_base)?;
    let source = Source {
        file,
        text: yaml,
        line_base,
    };
    let mut diags = Vec::new();
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
    if diags.is_empty() {
        Ok(parsed)
    } else {
        Err(diags)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CastFile {
    #[serde(rename = "schema")]
    _schema: String,
    pub(super) id: String,
    pub(super) kind: String,
    pub(super) display_name: String,
    pub(super) facts: Vec<FactFile>,
    #[serde(default)]
    pub(super) tick: Option<TickProfile>,
    #[serde(default)]
    pub(super) rules: Vec<RuleEntry>,
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
