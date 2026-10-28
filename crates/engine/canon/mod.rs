//! The loaded pack: places, cast, facts, and rules.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::Deserialize;

use crate::canon::tick::load_round_settings;
use crate::check::CheckDef;
use crate::diagnostic::{schema as schema_error, Diagnostic};
use crate::error::Reason;
use crate::hash::blake2s_hex;
use crate::meter::MeterDef;
use crate::rule::{Cond, Cost, Owner, Rule, RuleTable, TargetKind, When};
pub use spookery_protocol::{CastKind, MenuRow, PackSummary};

mod cast;
mod checks;
mod fact;
mod item;
mod lint;
mod meters;
#[cfg(test)]
mod oracle;
mod pack;
mod place;
mod rule_list;
mod save;
mod source;
mod story;
pub(crate) mod tick;

use cast::load_cast;
use checks::load_checks;
use fact::{fact_ids_of, insert_fact, FactFile};
use item::load_item;
pub(crate) use lint::check_canon;
use lint::{
    broken_exits, find_bare_resist, reveal_problems, unknown_checks, unknown_exprs, unknown_meters,
    unknown_rule_items,
};
use meters::load_meters;
use pack::load_pack;
use place::{compile_exits, load_place};
use rule_list::load_verbs;
pub(crate) use save::{check_save, migrate_save, pending_meters};
use source::{collect_files, read_text, split_frontmatter, At, Problem, Source};
use story::load_story;
pub use tick::Budget;

#[derive(Clone, Debug)]
pub struct Canon {
    pub pack_id: String,
    pub title: String,
    pub system: String,
    pub start_place: String,
    pub start_pc: String,
    pub places: BTreeMap<String, PlaceCanon>,
    pub cast: BTreeMap<String, CastCanon>,
    pub facts: BTreeMap<String, FactCanon>,
    pub rules: RuleTable,
    pub checks: BTreeMap<String, CheckDef>,
    pub meters: BTreeMap<String, MeterDef>,
    pub items: BTreeMap<String, ItemCanon>,
    pub budget: Budget,
    /// Ticks between journal checkpoints. `save.yaml` is rewritten then.
    pub checkpoint_every: u64,
}

#[derive(Clone, Debug)]
pub struct ItemCanon {
    pub name: String,
    pub tags: Vec<String>,
    pub body: String,
}

#[derive(Clone, Debug)]
pub struct FactCanon {
    pub text: String,
    pub hidden: bool,
    pub owner_id: String,
}

/// One way out of a place. `minutes` is the world time the walk takes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exit {
    pub name: String,
    pub to: String,
    pub minutes: u64,
    pub cost: Cost,
    pub if_: Vec<Cond>,
    pub locked_text: String,
    pub hidden: bool,
    pub revealed_by: Option<String>,
    pub file: String,
    pub index: usize,
}

#[derive(Clone, Debug)]
pub struct PlaceCanon {
    pub name: String,
    pub facts: Vec<String>,
    pub body: String,
    pub exits: Vec<Exit>,
}

#[derive(Clone, Debug)]
pub struct CastCanon {
    pub kind: CastKind,
    pub display_name: String,
    pub facts: Vec<String>,
    pub body: String,
    pub tick: Option<TickProfile>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TickProfile {
    pub eligible: bool,
    #[serde(default)]
    pub flags: Vec<String>,
}

impl Canon {
    pub fn pack_summary(&self) -> PackSummary {
        let menu = self
            .rules
            .iter()
            .filter_map(|rule| match &rule.when {
                When::Verb { verb, target } if matches!(rule.owner, Owner::Verb) => {
                    Some((verb.clone(), "verb", target.word()))
                }
                When::Habit => Some((rule.local_id.clone(), "habit", TargetKind::None.word())),
                _ => None,
            })
            .map(|(name, audience, token)| MenuRow {
                name,
                audience: audience.to_string(),
                token: token.to_string(),
                on: "self".to_string(),
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

    pub fn profile(&self, actor_id: &str) -> Option<&TickProfile> {
        self.cast.get(actor_id).and_then(|cast| cast.tick.as_ref())
    }

    pub fn display_name(&self, id: &str) -> String {
        self.cast
            .get(id)
            .map(|cast| cast.display_name.clone())
            .filter(|name| !name.is_empty())
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

pub fn load_canon(root: &Path) -> Result<Canon, Vec<Diagnostic>> {
    let mut diags = Vec::new();
    let pack = match read_text(root, "pack.yaml") {
        Ok(text) => match load_pack(&text) {
            Ok(pack) => Some(pack),
            Err(more) => {
                diags.extend(more);
                None
            }
        },
        Err(more) => {
            diags.extend(more);
            None
        }
    };
    let mut places = BTreeMap::new();
    let mut facts = BTreeMap::new();
    let mut items = BTreeMap::new();
    let mut loaded: Vec<Rule> = Vec::new();
    let mut place_files = Vec::new();
    if collect_files(&root.join("world"), root, &mut place_files).is_err() {
        diags.push(
            Problem {
                source: &Source::named("world"),
                path: "world",
                at: At::Nowhere,
                problem: "could not read world/".into(),
                hint: "check that the directory is readable",
            }
            .schema(),
        );
    }
    for (rel, bytes) in place_files {
        if !rel.ends_with(".md") {
            continue;
        }
        let Ok(text) = std::str::from_utf8(&bytes) else {
            diags.push(
                schema_error(&rel, "this file is not UTF-8 text", "save it as UTF-8")
                    .with_path("schema"),
            );
            continue;
        };
        let (yaml, body) = match split_frontmatter(&rel, text) {
            Ok(split) => split,
            Err(more) => {
                diags.extend(more);
                continue;
            }
        };
        if rel.starts_with("world/items/") {
            match load_item(&rel, yaml) {
                Ok(item) => {
                    if item.id.is_empty() {
                        diags.push(schema_error(
                            &rel,
                            "an item id is empty",
                            "set id to a short name",
                        ));
                        continue;
                    }
                    if items.contains_key(&item.id) {
                        diags.push(schema_error(
                            &rel,
                            &format!("item '{}' is already defined", item.id),
                            "keep one item with this id",
                        ));
                        continue;
                    }
                    let id = item.id.clone();
                    match rule_list::compile(&rel, item.rules, &Owner::Item(id.clone())) {
                        Ok(rules) => loaded.extend(rules),
                        Err(more) => diags.extend(more),
                    }
                    items.insert(
                        id,
                        ItemCanon {
                            name: item.name,
                            tags: item.tags,
                            body: body.trim().to_string(),
                        },
                    );
                }
                Err(more) => diags.extend(more),
            }
            continue;
        }
        if !rel.starts_with("world/places/") {
            continue;
        }
        let file = match load_place(&rel, yaml, 1) {
            Ok(file) => file,
            Err(more) => {
                diags.extend(more);
                continue;
            }
        };
        let mut fact_ok = true;
        for (index, fact) in file.facts.into_iter().enumerate() {
            if let Err(more) = insert_fact(&mut facts, &rel, text, index, fact, &file.id) {
                diags.extend(more);
                fact_ok = false;
            }
        }
        if !fact_ok {
            continue;
        }
        let ids = fact_ids_of(&facts, &file.id);
        let exits = match compile_exits(&rel, &file.exits) {
            Ok(exits) => exits,
            Err(more) => {
                diags.extend(more);
                Vec::new()
            }
        };
        if places.contains_key(&file.id) {
            diags.push(
                schema_error(
                    &rel,
                    &format!("place '{}' is already defined", file.id),
                    "keep one place with this id",
                )
                .with_path("id"),
            );
            continue;
        }
        places.insert(
            file.id.clone(),
            PlaceCanon {
                name: file.display_name,
                facts: ids,
                body: body.to_string(),
                exits,
            },
        );
        match rule_list::compile(&rel, file.rules, &Owner::Place(file.id)) {
            Ok(rules) => loaded.extend(rules),
            Err(more) => diags.extend(more),
        }
    }
    let mut cast = BTreeMap::new();
    let mut cast_files = Vec::new();
    if collect_files(&root.join("cast"), root, &mut cast_files).is_err() {
        diags.push(
            Problem {
                source: &Source::named("cast"),
                path: "cast",
                at: At::Nowhere,
                problem: "could not read cast/".into(),
                hint: "check that the directory is readable",
            }
            .schema(),
        );
    }
    for (rel, bytes) in cast_files {
        if !rel.ends_with(".md") {
            continue;
        }
        let Ok(text) = std::str::from_utf8(&bytes) else {
            diags.push(
                schema_error(&rel, "this file is not UTF-8 text", "save it as UTF-8")
                    .with_path("schema"),
            );
            continue;
        };
        let (yaml, body) = match split_frontmatter(&rel, text) {
            Ok(split) => split,
            Err(more) => {
                diags.extend(more);
                continue;
            }
        };
        let file = match load_cast(&rel, yaml, 1) {
            Ok(file) => file,
            Err(more) => {
                diags.extend(more);
                continue;
            }
        };
        let mut fact_ok = true;
        for (index, fact) in file.facts.iter().enumerate() {
            if let Err(more) = insert_fact(
                &mut facts,
                &rel,
                text,
                index,
                FactFile {
                    id: fact.id.clone(),
                    text: fact.text.clone(),
                    hidden: fact.hidden,
                },
                &file.id,
            ) {
                diags.extend(more);
                fact_ok = false;
            }
        }
        if !fact_ok {
            continue;
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
                    &rel,
                    &format!("cast id '{}' is already defined", file.id),
                    "keep one cast file with this id",
                )
                .with_path("id"),
            );
            continue;
        }
        let id = file.id.clone();
        cast.insert(
            id.clone(),
            CastCanon {
                kind,
                display_name: file.display_name,
                facts: fact_ids_of(&facts, &id),
                body: body.to_string(),
                tick: file.tick,
            },
        );
        match rule_list::compile(&rel, file.rules, &Owner::Cast(id)) {
            Ok(rules) => loaded.extend(rules),
            Err(more) => diags.extend(more),
        }
    }
    if root.join("rules/moves.yaml").is_file() {
        diags.push(rule_list::replaced_moves_file());
    }
    let verbs_text = match read_text(root, "rules/verbs.yaml") {
        Ok(text) => Some(text),
        Err(more) => {
            diags.extend(more);
            None
        }
    };
    let checks_text = match read_text(root, "rules/checks.yaml") {
        Ok(text) => Some(text),
        Err(more) => {
            diags.extend(more);
            None
        }
    };
    let meters_text = match read_text(root, "rules/meters.yaml") {
        Ok(text) => Some(text),
        Err(more) => {
            diags.extend(more);
            None
        }
    };
    let tick_text = match read_text(root, "rules/tick.yaml") {
        Ok(text) => Some(text),
        Err(more) => {
            diags.extend(more);
            None
        }
    };
    if let Some(text) = &verbs_text {
        match load_verbs(text) {
            Ok(rules) => loaded.extend(rules),
            Err(more) => diags.extend(more),
        }
    }
    let mut story_files = Vec::new();
    if root.join("story").is_dir()
        && collect_files(&root.join("story"), root, &mut story_files).is_err()
    {
        diags.push(
            Problem {
                source: &Source::named("story"),
                path: "story",
                at: At::Nowhere,
                problem: "could not read story/".into(),
                hint: "check that the directory is readable",
            }
            .schema(),
        );
    }
    for (rel, bytes) in story_files {
        if !rel.ends_with(".yaml") {
            continue;
        }
        let Ok(text) = std::str::from_utf8(&bytes) else {
            diags.push(schema_error(
                &rel,
                "this file is not UTF-8 text",
                "save it as UTF-8",
            ));
            continue;
        };
        match load_story(&rel, text) {
            Ok(story) => match rule_list::compile(&rel, story.rules, &Owner::Story(story.id)) {
                Ok(rules) => loaded.extend(rules),
                Err(more) => diags.extend(more),
            },
            Err(more) => diags.extend(more),
        }
    }
    let mut seen_ids = BTreeSet::new();
    for rule in &loaded {
        if !seen_ids.insert(rule.id.clone()) {
            diags.push(schema_error(
                &rule.file,
                &format!("rule '{}' is already defined", rule.id),
                "keep one rule with this id",
            ));
        }
    }
    let rules = RuleTable::new(loaded);
    let checks = match &checks_text {
        Some(text) => match load_checks(text) {
            Ok(checks) => Some(checks),
            Err(more) => {
                diags.extend(more);
                None
            }
        },
        None => None,
    };
    let meters = match &meters_text {
        Some(text) => match load_meters(text) {
            Ok(meters) => Some(meters),
            Err(more) => {
                diags.extend(more);
                None
            }
        },
        None => None,
    };
    let budget = match &tick_text {
        Some(text) => match load_round_settings(text) {
            Ok(budget) => Some(budget),
            Err(more) => {
                diags.extend(more);
                None
            }
        },
        None => None,
    };
    if let Some(checks) = &checks {
        diags.extend(unknown_checks(&rules, checks));
    }
    if let (Some(meters), Some(checks)) = (&meters, &checks) {
        diags.extend(unknown_meters(meters, checks, &rules));
    }
    if let Some(meters) = &meters {
        diags.extend(unknown_exprs(meters, &cast, &places, &rules));
    }
    diags.extend(unknown_rule_items(&rules, &items));
    diags.extend(reveal_problems(&rules, &facts));
    diags.extend(find_bare_resist(&rules));
    diags.extend(broken_exits(&places, &facts, &items));
    let checkpoint_every = match pack.as_ref().and_then(|file| file.checkpoint_every) {
        None => 100,
        Some(0) => {
            diags.push(
                schema_error(
                    "pack.yaml",
                    "checkpoint_every is 0",
                    "set checkpoint_every to a positive number of ticks, or omit it",
                )
                .with_path("checkpoint_every"),
            );
            100
        }
        Some(every) => every,
    };
    if !diags.is_empty() {
        return Err(diags);
    }
    let pack = pack.expect("pack loaded");
    Ok(Canon {
        pack_id: pack.id,
        title: pack.title,
        system: pack.system,
        start_place: pack.start.place,
        start_pc: pack.start.pc,
        places,
        cast,
        facts,
        rules,
        checks: checks.expect("checks loaded"),
        meters: meters.expect("meters loaded"),
        items,
        budget: budget.expect("tick loaded"),
        checkpoint_every,
    })
}

/// Markdown with the body removed. Frontmatter stays. The trailing delimiter is kept.
#[allow(dead_code)]
pub fn strip_body(text: &str) -> Result<String, Reason> {
    let (yaml, _) = split_frontmatter("", text).map_err(|_| Reason::Schema)?;
    Ok(format!("---\n{yaml}\n---\n"))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::diagnostic::reason_of;
    use crate::error::Reason;

    #[test]
    fn docks_pack_rev_matches_now_yaml() {
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
        assert_eq!(reason_of(old_err), Reason::Schema);
        let bogus = load(&text("bogus")).unwrap_err();
        assert!(bogus[0].message.contains("does not use"), "{bogus:?}");
        assert_eq!(reason_of(bogus), Reason::Schema);
    }

    #[test]
    fn strip_body_rejects_a_missing_fence() {
        assert_eq!(strip_body("no fence\n"), Err(Reason::Schema));
        assert_eq!(strip_body("---\nno closer\n"), Err(Reason::Schema));
    }

    #[test]
    fn menu_rows_keep_player_and_habit_and_kinds() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packs/oneshot-docks");
        let mut canon = load_canon(&root).expect("docks canon");
        canon.cast.get_mut("mira").unwrap().body = "CAST-BODY-SENTINEL".to_string();
        let summary = canon.pack_summary();
        assert!(summary.menu.iter().all(|row| row.audience != "system"));
        assert!(summary.menu.iter().any(|row| row.name == "travel"
            && row.audience == "verb"
            && row.token == "exit"
            && row.on == "self"));
        assert!(summary
            .menu
            .iter()
            .any(|row| row.name == "restock_lamp_oil" && row.audience == "habit"));
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
