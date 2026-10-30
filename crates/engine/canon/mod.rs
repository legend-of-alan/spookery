//! The loaded pack: places, cast, facts, and rules.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::Deserialize;

use crate::diagnostic::Diagnostic;
use crate::error::Reason;
use crate::hash::blake2s_hex;
use crate::meter::MeterDef;
use crate::rule::{Cond, Cost, Owner, RuleTable, TargetKind, When};
use crate::rules::roll::Check;
pub use spookery_protocol::{CastKind, MenuRow, PackSummary};

mod budget;
mod cast;
mod checks;
mod fact;
mod item;
mod lint;
mod meters;
mod pack;
mod place;
mod rule_list;
mod save;
mod source;
mod story;

use budget::load_budget;
pub(crate) use budget::{Budget, PausePolicy};
use cast::add_cast;
use checks::load_checks;
use fact::Facts;
use item::add_item;
pub(crate) use lint::check_canon;
use lint::{
    broken_exits, resist_same_without_roll, reveal_problems, unknown_checks, unknown_meters,
    unknown_reads, unknown_rule_items,
};
use meters::load_meters;
use pack::load_pack_yaml;
use place::add_place;
use rule_list::load_verbs;
pub(crate) use save::{check_save, migrate_save, missing_meters};
use source::{collect_files, collect_markdown_files, read_text, Diags, FileText, Locator};
use story::load_stories;

#[derive(Clone, Debug)]
pub struct Canon {
    pub pack_id: String,
    pub title: String,
    pub system: String,
    pub start_place: String,
    pub start_pc: String,
    pub places: BTreeMap<String, PlaceDef>,
    pub cast: BTreeMap<String, CastDef>,
    pub facts: BTreeMap<String, FactDef>,
    pub rules: RuleTable,
    pub checks: BTreeMap<String, Check>,
    pub meters: BTreeMap<String, MeterDef>,
    pub items: BTreeMap<String, ItemDef>,
    pub budget: Budget,
    /// Rounds between journal checkpoints. `save.yaml` is rewritten then.
    pub checkpoint_every: u64,
}

#[derive(Clone, Debug)]
pub struct ItemDef {
    pub name: String,
    pub tags: Vec<String>,
    pub body: String,
}

#[derive(Clone, Debug)]
pub struct FactDef {
    pub text: String,
    pub hidden: bool,
    pub owner_id: String,
}

/// One way out of a place. `cost.minutes` is the world time the walk takes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exit {
    pub name: String,
    pub to: String,
    pub cost: Cost,
    pub if_: Vec<Cond>,
    pub locked_text: String,
    pub hidden: bool,
    pub revealed_by: Option<String>,
    pub file: String,
    pub index: usize,
}

#[derive(Clone, Debug)]
pub struct PlaceDef {
    pub display_name: String,
    pub facts: Vec<String>,
    pub body: String,
    pub exits: Vec<Exit>,
}

#[derive(Clone, Debug)]
pub struct CastDef {
    pub kind: CastKind,
    pub display_name: String,
    pub facts: Vec<String>,
    /// The flags an effect may set on this actor.
    pub flags: Vec<String>,
    pub body: String,
    pub idle: Option<IdleProfile>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdleProfile {
    pub eligible: bool,
}

impl Canon {
    pub fn to_pack_summary(&self) -> PackSummary {
        let menu = self
            .rules
            .iter()
            .filter_map(|rule| match &rule.when {
                When::Verb { verb, target } if matches!(rule.owner, Owner::Verb) => {
                    Some((verb.clone(), "verb", target.as_str()))
                }
                When::Habit => Some((rule.local_id.clone(), "habit", TargetKind::None.as_str())),
                _ => None,
            })
            .map(|(name, audience, token)| MenuRow {
                name,
                kind: audience.to_string(),
                target: token.to_string(),
            })
            .collect();
        let kinds = self.cast.iter().map(|(k, m)| (k.clone(), m.kind)).collect();
        let clip = |s: &str| s.chars().take(800).collect();
        let places = self
            .places
            .iter()
            .map(|(k, p)| (k.clone(), clip(&p.body)))
            .collect();
        let cast = self
            .cast
            .iter()
            .map(|(k, m)| (k.clone(), clip(&m.body)))
            .collect();
        PackSummary {
            menu,
            kinds,
            places,
            cast,
        }
    }

    pub fn idle_profile(&self, actor_id: &str) -> Option<&IdleProfile> {
        self.cast.get(actor_id).and_then(|cast| cast.idle.as_ref())
    }

    pub fn display_name(&self, id: &str) -> String {
        self.cast
            .get(id)
            .map(|cast| cast.display_name.clone())
            .unwrap_or_else(|| id.to_string())
    }

    pub(crate) fn body_of_fact(&self, fact_id: &str) -> &str {
        let Some(fact) = self.facts.get(fact_id) else {
            return "";
        };
        if let Some(place) = self.places.get(&fact.owner_id) {
            return &place.body;
        }
        if let Some(cast) = self.cast.get(&fact.owner_id) {
            return &cast.body;
        }
        ""
    }
}

pub fn pack_rev(root: &Path) -> Result<String, Reason> {
    let mut files = Vec::new();
    for dir in ["world", "cast", "rules"] {
        collect_files(&root.join(dir), root, &mut files)?;
    }
    files.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    let mut preimage = Vec::new();
    for (rel, bytes) in files {
        preimage.extend(rel.as_bytes());
        preimage.push(0);
        preimage.extend((bytes.len() as u64).to_be_bytes());
        preimage.extend(bytes);
    }
    Ok(format!("blake2s:{}", blake2s_hex(&preimage)))
}

/// Loads a pack in the numbered order below, which is also the order of its diagnostics.
pub fn load_canon(root: &Path) -> Result<Canon, Vec<Diagnostic>> {
    let mut diags = Diags::default();
    let mut places = BTreeMap::new();
    let mut cast = BTreeMap::new();
    let mut facts = Facts::default();
    let mut items = BTreeMap::new();
    let mut loaded = Vec::new();
    // 1. pack.yaml: missing, unreadable, tag, shape, empty id.
    let pack = diags.keep(read_text(root, "pack.yaml").and_then(|text| load_pack_yaml(&text)));
    // 2. One walk of world/: items and places by path prefix; other files are skipped.
    for page in collect_markdown_files(root, "world") {
        let Some(page) = diags.keep(page) else {
            continue;
        };
        let rules = if page.file.starts_with("world/items/") {
            add_item(page, &mut items, &mut diags)
        } else if page.file.starts_with("world/places/") {
            add_place(page, &mut facts, &mut places, &mut diags)
        } else {
            None
        };
        loaded.extend(rules.unwrap_or_default());
    }
    // 3. One walk of cast/.
    for page in collect_markdown_files(root, "cast") {
        let Some(page) = diags.keep(page) else {
            continue;
        };
        loaded.extend(add_cast(page, &mut facts, &mut cast, &mut diags).unwrap_or_default());
    }
    // 4. rules/moves.yaml is refused.
    if root.join("rules/moves.yaml").is_file() {
        diags.push(rule_list::replaced_moves_file());
    }
    // 5. The four rule files are read in this order; a missing one is reported here.
    let [verbs, checks, meters, budget] = [
        "rules/verbs.yaml",
        "rules/checks.yaml",
        "rules/meters.yaml",
        "rules/budget.yaml",
    ]
    .map(|rel| diags.keep(read_text(root, rel)).map(|text| (rel, text)));
    // 6. Verbs.
    if let Some((rel, text)) = &verbs {
        loaded.extend(diags.keep(load_verbs(rel, text)).unwrap_or_default());
    }
    // 7. story/.
    loaded.extend(load_stories(root, &mut diags));
    // 8. Rule ids across files.
    let mut seen_ids = BTreeSet::new();
    for rule in &loaded {
        let twice = ("rule", "is already defined", "keep one rule with this id");
        let taken = !seen_ids.insert(rule.id.clone());
        let source = FileText::named(&rule.file);
        diags.extend(taken.then(|| source.refuse_duplicate(&rule.id, "", Locator::Nowhere, twice)));
    }
    let rules = RuleTable::new(loaded);
    // 9. Checks. 10. Meters. 11. Budget.
    let checks = checks.and_then(|(rel, text)| diags.keep(load_checks(rel, &text)));
    let meters = meters.and_then(|(rel, text)| diags.keep(load_meters(rel, &text)));
    let budget = budget.and_then(|(rel, text)| diags.keep(load_budget(rel, &text)));
    // 12. Lints across files.
    if let Some(checks) = &checks {
        diags.extend(unknown_checks(&rules, checks));
    }
    if let (Some(meters), Some(checks)) = (&meters, &checks) {
        diags.extend(unknown_meters(meters, checks, &rules));
    }
    if let Some(meters) = &meters {
        diags.extend(unknown_reads(meters, &cast, &places, &rules));
    }
    diags.extend(unknown_rule_items(&rules, &items));
    diags.extend(reveal_problems(&rules, &facts.0));
    diags.extend(resist_same_without_roll(&rules));
    diags.extend(broken_exits(&places, &facts.0, &items));
    // 13. checkpoint_every.
    let checkpoint_every = pack
        .as_ref()
        .and_then(|pack| diags.keep(pack.checkpoint_every()));
    diags.into_result(())?;
    let (Some(pack), Some(checks), Some(meters), Some(budget), Some(checkpoint_every)) =
        (pack, checks, meters, budget, checkpoint_every)
    else {
        unreachable!("a part that did not load reported why");
    };
    Ok(Canon {
        pack_id: pack.id,
        title: pack.title,
        system: pack.system,
        start_place: pack.start.place,
        start_pc: pack.start.pc,
        places,
        cast,
        facts: facts.0,
        rules,
        checks,
        meters,
        items,
        budget,
        checkpoint_every,
    })
}

/// Markdown with the body removed. Frontmatter stays. The trailing delimiter is kept.
#[cfg(any(test, feature = "testing"))]
pub fn strip_body(text: &str) -> Result<String, Reason> {
    let (yaml, _) = source::split_frontmatter("", text).map_err(|_| Reason::Schema)?;
    Ok(format!("---\n{yaml}\n---\n"))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::diagnostic::first_reason;
    use crate::error::Reason;

    #[test]
    fn docks_pack_rev_matches_save_yaml() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packs/oneshot-docks");
        let rev = pack_rev(&root).unwrap();
        let text = fs::read_to_string(root.join("state/save.yaml")).unwrap();
        let line = text
            .lines()
            .find(|line| line.starts_with("pack_rev:"))
            .unwrap();
        assert_eq!(line, format!("pack_rev: {rev}"));
    }

    pub(super) fn rejects(
        load: fn(&str) -> Result<(), Vec<Diagnostic>>,
        good: &str,
        old: &str,
        body: &str,
    ) {
        let text = |tag: &str| format!("schema: {tag}\n{body}");
        assert!(load(&text(good)).is_ok());
        let old_err = load(&text(old)).unwrap_err();
        assert!(old_err[0].message.contains("no version"), "{old_err:?}");
        assert_eq!(first_reason(&old_err), Reason::Schema);
        let bogus = load(&text("bogus")).unwrap_err();
        assert!(bogus[0].message.contains("does not use"), "{bogus:?}");
        assert_eq!(first_reason(&bogus), Reason::Schema);
    }

    #[test]
    fn strip_body_rejects_a_missing_fence() {
        assert_eq!(strip_body("no fence\n"), Err(Reason::Schema));
        assert_eq!(strip_body("---\nno closer\n"), Err(Reason::Schema));
    }

    #[test]
    fn menu_rows_keep_verbs_and_habits_and_kinds() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packs/oneshot-docks");
        let mut canon = load_canon(&root).expect("docks canon");
        canon.cast.get_mut("mira").unwrap().body = "CAST-BODY-SENTINEL".to_string();
        let summary = canon.to_pack_summary();
        assert!(summary.menu.iter().all(|row| row.kind != "system"));
        assert!(summary
            .menu
            .iter()
            .any(|row| row.name == "travel" && row.kind == "verb" && row.target == "exit"));
        assert!(summary
            .menu
            .iter()
            .any(|row| row.name == "restock_lamp_oil" && row.kind == "habit"));
        assert_eq!(summary.kinds.get("mira"), Some(&CastKind::Npc));
        assert_eq!(summary.kinds.get("pc_rowan"), Some(&CastKind::PcTemplate));
        let rendered = format!("{:?}{:?}", summary.menu, summary.kinds);
        assert!(!rendered.contains("CAST-BODY-SENTINEL"));
        assert_eq!(
            summary.cast.get("mira").map(String::as_str),
            Some("CAST-BODY-SENTINEL")
        );
    }
}
