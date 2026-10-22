//! The loaded pack: places, cast, facts, and rules.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::canon::world_settings::{load_world_settings, Budget};
use crate::check::{load_checks, CheckDef};
use crate::diagnostic::{
    coded, locate_nth, locate_row_field, parse_file, schema as schema_error, tag_diagnostic,
    with_spot, Diagnostic,
};
use crate::error::Reason;
use crate::format;
use crate::hash::blake2s_hex;
use crate::meter::MeterDef;
use crate::rule::{Cond, Cost, Owner, Rule, RuleTable};

mod check;
mod save;
pub mod schema;
pub mod world_settings;

use check::{
    broken_exits, find_bare_resist, reveal_problems, unknown_checks, unknown_exprs, unknown_meters,
    unknown_rule_items,
};
pub(crate) use save::pending_meters;
pub use save::{check_save, migrate_save};
use schema::RuleEntry;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CastKind {
    Npc,
    PcTemplate,
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
    if let Err(reason) = collect_files(&root.join("world"), root, &mut place_files) {
        diags.push(io_schema_error("world", reason, "could not read world/"));
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
                    match schema::compile(&rel, item.rules, &Owner::Item(id.clone())) {
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
        match schema::compile(&rel, file.rules, &Owner::Place(file.id)) {
            Ok(rules) => loaded.extend(rules),
            Err(more) => diags.extend(more),
        }
    }
    let mut cast = BTreeMap::new();
    let mut cast_files = Vec::new();
    if let Err(reason) = collect_files(&root.join("cast"), root, &mut cast_files) {
        diags.push(io_schema_error("cast", reason, "could not read cast/"));
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
        match schema::compile(&rel, file.rules, &Owner::Cast(id)) {
            Ok(rules) => loaded.extend(rules),
            Err(more) => diags.extend(more),
        }
    }
    if root.join("rules/moves.yaml").is_file() {
        diags.push(schema::replaced_moves_file());
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
    if root.join("story").is_dir() {
        if let Err(reason) = collect_files(&root.join("story"), root, &mut story_files) {
            diags.push(io_schema_error("story", reason, "could not read story/"));
        }
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
            Ok(story) => match schema::compile(&rel, story.rules, &Owner::Story(story.id)) {
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
        Some(text) => match load_world_settings(text) {
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

fn io_schema_error(file: &str, reason: Reason, problem: &str) -> Diagnostic {
    coded(
        file,
        reason,
        problem,
        "check that the directory is readable",
    )
    .with_path(file)
}

fn fact_ids_of(facts: &BTreeMap<String, FactCanon>, owner: &str) -> Vec<String> {
    facts
        .iter()
        .filter(|(_, fact)| fact.owner_id == owner)
        .map(|(id, _)| id.clone())
        .collect()
}

fn insert_fact(
    facts: &mut BTreeMap<String, FactCanon>,
    file: &str,
    text: &str,
    index: usize,
    fact: FactFile,
    owner: &str,
) -> Result<(), Vec<Diagnostic>> {
    if fact.id.is_empty() {
        return Err(vec![fact_schema_error(
            file,
            text,
            index,
            "id:",
            "a fact id is empty",
            "set id to a short name",
        )]);
    }
    if facts.contains_key(&fact.id) {
        return Err(vec![fact_schema_error(
            file,
            text,
            index,
            "id:",
            &format!("fact '{}' is already defined", fact.id),
            "keep one fact with this id",
        )]);
    }
    facts.insert(
        fact.id,
        FactCanon {
            text: fact.text,
            hidden: fact.hidden,
            owner_id: owner.to_string(),
        },
    );
    Ok(())
}

fn fact_schema_error(
    file: &str,
    text: &str,
    index: usize,
    field: &str,
    problem: &str,
    hint: &str,
) -> Diagnostic {
    let mut diag = schema_error(file, problem, hint).with_path(format!("facts[{index}].id"));
    if let Some((line, column)) = locate_row_field(text, "id:", index, field) {
        diag = diag.at_line(line, Some(column));
    }
    diag
}

fn compile_exits(file: &str, exits: &[ExitFile]) -> Result<Vec<Exit>, Vec<Diagnostic>> {
    let mut diags = Vec::new();
    let mut compiled = Vec::new();
    let mut names = BTreeSet::new();
    for (index, exit) in exits.iter().enumerate() {
        if exit.name.is_empty() || exit.to.is_empty() {
            diags.push(
                schema_error(file, "an exit needs a name and a to", "set both")
                    .with_path(format!("exits[{index}]")),
            );
            continue;
        }
        if !names.insert(exit.name.clone()) {
            diags.push(
                schema_error(
                    file,
                    &format!("exit '{}' is listed twice", exit.name),
                    "keep one exit with this name",
                )
                .with_path(format!("exits[{index}].name")),
            );
            continue;
        }
        let cost = match &exit.cost {
            Some(yaml) => match schema::cost_of(yaml) {
                Ok(cost) => cost,
                Err(fault) => {
                    let mut diag = schema_error(file, &fault.message, "fix the exit cost")
                        .with_path(format!("exits[{index}].cost"));
                    if let Some(line) = fault.line {
                        diag = diag.at_line(line, fault.column);
                    }
                    diags.push(diag);
                    continue;
                }
            },
            None => Cost {
                minutes: 0,
                meters: Vec::new(),
                items: Vec::new(),
            },
        };
        let mut if_ = Vec::new();
        let mut bad = false;
        for cond in &exit.if_ {
            match schema::cond_of(cond) {
                Ok(cond) => if_.push(cond),
                Err(fault) => {
                    let mut diag = schema_error(file, &fault.message, "fix the exit condition")
                        .with_path(format!("exits[{index}].if"));
                    if let Some(line) = fault.line {
                        diag = diag.at_line(line, fault.column);
                    }
                    diags.push(diag);
                    bad = true;
                }
            }
        }
        if bad {
            continue;
        }
        compiled.push(Exit {
            name: exit.name.clone(),
            to: exit.to.clone(),
            minutes: cost.minutes,
            cost,
            if_,
            locked_text: exit.locked_text.clone(),
            hidden: exit.hidden,
            revealed_by: exit.revealed_by.clone(),
            file: file.to_string(),
            index,
        });
    }
    if diags.is_empty() {
        Ok(compiled)
    } else {
        Err(diags)
    }
}

pub(crate) fn load_meters(text: &str) -> Result<BTreeMap<String, MeterDef>, Vec<Diagnostic>> {
    const FILE: &str = "rules/meters.yaml";
    let file = load_headed(FILE, text, format::METERS, 0, |file: MetersFile| {
        if file.schema != format::METERS {
            return Err(vec![schema_error(
                FILE,
                &format!("schema is '{}', which this file does not use", file.schema),
                "write 'schema: meters' with no version suffix",
            )
            .with_path("schema")]);
        }
        Ok(file)
    })?;
    let mut diags = Vec::new();
    let mut meters = BTreeMap::new();
    for (index, entry) in file.meters.into_iter().enumerate() {
        let path = format!("meters[{index}]");
        let mut bad = false;
        if entry.id.is_empty() {
            diags.push(meter_schema_error(
                text,
                index,
                "id:",
                &format!("{path}.id"),
                "a meter id is empty",
                "set id to a short name",
            ));
            bad = true;
        }
        if entry.min > entry.base_cap {
            diags.push(meter_schema_error(
                text,
                index,
                "min:",
                &format!("{path}.min"),
                &format!("meter '{}' has min above base_cap", entry.id),
                "set min at or below base_cap",
            ));
            bad = true;
        }
        if entry.base_cap > entry.max {
            diags.push(meter_schema_error(
                text,
                index,
                "base_cap:",
                &format!("{path}.base_cap"),
                &format!("meter '{}' has base_cap above max", entry.id),
                "set base_cap at or below max",
            ));
            bad = true;
        }
        if !(entry.min..=entry.base_cap).contains(&entry.start) {
            diags.push(meter_schema_error(
                text,
                index,
                "start:",
                &format!("{path}.start"),
                &format!("meter '{}' has start outside min and base_cap", entry.id),
                "set start between min and base_cap",
            ));
            bad = true;
        }
        if !bad && meters.contains_key(&entry.id) {
            diags.push(meter_schema_error(
                text,
                index,
                "id:",
                &format!("{path}.id"),
                &format!("meter '{}' is listed twice", entry.id),
                "keep one meter",
            ));
            bad = true;
        }
        if !bad {
            meters.insert(
                entry.id,
                MeterDef {
                    min: entry.min,
                    base_cap: entry.base_cap,
                    max: entry.max,
                    start: entry.start,
                    hidden: entry.hidden,
                },
            );
        }
    }
    if diags.is_empty() {
        Ok(meters)
    } else {
        Err(diags)
    }
}

fn meter_schema_error(
    text: &str,
    index: usize,
    field: &str,
    path: &str,
    problem: &str,
    hint: &str,
) -> Diagnostic {
    let mut diag = schema_error("rules/meters.yaml", problem, hint).with_path(path);
    if let Some((line, column)) = locate_row_field(text, "id:", index, field) {
        diag = diag.at_line(line, Some(column));
    }
    diag
}

fn load_verbs(text: &str) -> Result<Vec<Rule>, Vec<Diagnostic>> {
    const FILE: &str = "rules/verbs.yaml";
    let file = load_headed(FILE, text, format::VERBS, 0, |file: schema::VerbsFile| {
        if file.schema != format::VERBS {
            return Err(vec![schema_error(
                FILE,
                &format!("schema is '{}', which this file does not use", file.schema),
                "write 'schema: verbs' with no version suffix",
            )
            .with_path("schema")]);
        }
        Ok(file)
    })?;
    schema::compile(FILE, file.rules, &Owner::Verb)
}

fn load_item(file: &str, yaml: &str) -> Result<ItemFile, Vec<Diagnostic>> {
    load_headed(file, yaml, format::ITEM, 1, |item: ItemFile| {
        if item.schema != format::ITEM {
            return Err(vec![schema_error(
                file,
                &format!("schema is '{}', which this file does not use", item.schema),
                "write 'schema: item' with no version suffix",
            )]);
        }
        Ok(item)
    })
}

fn load_story(file: &str, text: &str) -> Result<StoryFile, Vec<Diagnostic>> {
    load_headed(file, text, format::STORY, 0, |story: StoryFile| {
        if story.schema != format::STORY {
            return Err(vec![schema_error(
                file,
                &format!("schema is '{}', which this file does not use", story.schema),
                "write 'schema: story' with no version suffix",
            )]);
        }
        Ok(story)
    })
}

fn read_text(root: &Path, rel: &str) -> Result<String, Vec<Diagnostic>> {
    let path = root.join(rel);
    if !path.is_file() {
        return Err(vec![coded(
            rel,
            Reason::NotInSpec,
            &format!("{rel} is missing"),
            &format!("add {rel}"),
        )
        .with_path("schema")]);
    }
    fs::read_to_string(&path).map_err(|_| {
        vec![coded(
            rel,
            Reason::Schema,
            "this file could not be read as UTF-8 text",
            "save it as UTF-8",
        )
        .with_path("schema")]
    })
}

fn load_pack(text: &str) -> Result<PackFile, Vec<Diagnostic>> {
    load_headed("pack.yaml", text, format::PACK, 0, |file: PackFile| {
        let mut diags = Vec::new();
        if file.schema != format::PACK {
            diags.push(
                schema_error(
                    "pack.yaml",
                    &format!("schema is '{}', which this file does not use", file.schema),
                    "write 'schema: pack' with no version suffix",
                )
                .with_path("schema"),
            );
        }
        if file.id.is_empty() {
            diags.push(
                schema_error("pack.yaml", "id is empty", "set id to a short pack id")
                    .with_path("id"),
            );
        }
        if diags.is_empty() {
            Ok(file)
        } else {
            Err(diags)
        }
    })
}

fn load_place(file: &str, yaml: &str, line_base: u32) -> Result<PlaceFile, Vec<Diagnostic>> {
    load_headed(file, yaml, format::PLACE, line_base, |parsed: PlaceFile| {
        let mut diags = Vec::new();
        if parsed.schema != format::PLACE {
            diags.push(headed_schema_error(
                file,
                yaml,
                line_base,
                "schema:",
                "schema",
                &format!(
                    "schema is '{}', which this file does not use",
                    parsed.schema
                ),
                "write 'schema: place' with no version suffix",
            ));
        }
        if parsed.id.is_empty() {
            diags.push(headed_schema_error(
                file,
                yaml,
                line_base,
                "id:",
                "id",
                "id is empty",
                "set id to a short place id",
            ));
        }
        if parsed.display_name.trim().is_empty() {
            diags.push(headed_schema_error(
                file,
                yaml,
                line_base,
                "display_name:",
                "display_name",
                "display_name is empty",
                "set display_name to the name players see",
            ));
        }
        if diags.is_empty() {
            Ok(parsed)
        } else {
            Err(diags)
        }
    })
}

fn load_cast(file: &str, yaml: &str, line_base: u32) -> Result<CastFile, Vec<Diagnostic>> {
    load_headed(file, yaml, format::CAST, line_base, |parsed: CastFile| {
        let mut diags = Vec::new();
        if parsed.schema != format::CAST {
            diags.push(headed_schema_error(
                file,
                yaml,
                line_base,
                "schema:",
                "schema",
                &format!(
                    "schema is '{}', which this file does not use",
                    parsed.schema
                ),
                "write 'schema: cast' with no version suffix",
            ));
        }
        if parsed.id.is_empty() {
            diags.push(headed_schema_error(
                file,
                yaml,
                line_base,
                "id:",
                "id",
                "id is empty",
                "set id to a short cast id",
            ));
        }
        if parsed.id == crate::effect::WORLD {
            diags.push(headed_schema_error(
                file,
                yaml,
                line_base,
                "id:",
                "id",
                "id world is reserved",
                "rename this cast member",
            ));
        }
        if parsed.display_name.trim().is_empty() {
            diags.push(headed_schema_error(
                file,
                yaml,
                line_base,
                "display_name:",
                "display_name",
                "display_name is empty",
                "set display_name to the name players see",
            ));
        }
        if parsed.kind != "npc" && parsed.kind != "pc_template" {
            diags.push(headed_schema_error(
                file,
                yaml,
                line_base,
                "kind:",
                "kind",
                &format!("kind is '{}', which is not npc or pc_template", parsed.kind),
                "set kind to npc or pc_template",
            ));
        }
        if diags.is_empty() {
            Ok(parsed)
        } else {
            Err(diags)
        }
    })
}

fn load_headed<T: serde::de::DeserializeOwned>(
    file: &str,
    text: &str,
    expected: &str,
    line_base: u32,
    check: impl FnOnce(T) -> Result<T, Vec<Diagnostic>>,
) -> Result<T, Vec<Diagnostic>> {
    if let Some(diag) = tag_diagnostic(text, expected, file, line_base) {
        return Err(vec![diag]);
    }
    check(parse_file(file, text, line_base)?)
}

fn headed_schema_error(
    file: &str,
    text: &str,
    line_base: u32,
    field: &str,
    path: &str,
    problem: &str,
    hint: &str,
) -> Diagnostic {
    with_spot(
        schema_error(file, problem, hint).with_path(path),
        locate_nth(text, field, 0),
        line_base,
    )
}

fn collect_files(dir: &Path, root: &Path, out: &mut Vec<(String, Vec<u8>)>) -> Result<(), Reason> {
    if !dir.exists() {
        return Ok(());
    }
    let mut entries = fs::read_dir(dir)
        .map_err(|_| Reason::Schema)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| Reason::Schema)?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, root, out)?;
            continue;
        }
        let rel = path.strip_prefix(root).map_err(|_| Reason::Schema)?;
        let rel = rel
            .components()
            .map(|component| component.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        let bytes = fs::read(&path).map_err(|_| Reason::Schema)?;
        out.push((rel, bytes));
    }
    Ok(())
}

fn split_frontmatter<'a>(file: &str, text: &'a str) -> Result<(&'a str, &'a str), Vec<Diagnostic>> {
    let rest = match text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
    {
        Some(rest) => rest,
        None => {
            return Err(vec![schema_error(
                file,
                "this file has no YAML frontmatter",
                "start with a --- line and close the header with ---",
            )
            .at_line(1, Some(1))]);
        }
    };
    if let Some(index) = rest.find("\n---\n") {
        return Ok((&rest[..index], &rest[index + 5..]));
    }
    if let Some(index) = rest.find("\n---\r\n") {
        return Ok((&rest[..index], &rest[index + 6..]));
    }
    if let Some(yaml) = rest.strip_suffix("\n---") {
        return Ok((yaml, ""));
    }
    Err(vec![schema_error(
        file,
        "this file has no closing frontmatter fence",
        "close the header with a --- line",
    )
    .at_line(1, Some(1))])
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PackFile {
    schema: String,
    id: String,
    title: String,
    system: String,
    start: PackStart,
    #[serde(default)]
    checkpoint_every: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PackStart {
    place: String,
    pc: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlaceFile {
    schema: String,
    id: String,
    display_name: String,
    facts: Vec<FactFile>,
    #[serde(default)]
    exits: Vec<ExitFile>,
    #[serde(default)]
    rules: Vec<RuleEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExitFile {
    to: String,
    name: String,
    #[serde(default)]
    cost: Option<schema::CostYaml>,
    #[serde(default, rename = "if")]
    if_: Vec<schema::CondYaml>,
    #[serde(default)]
    locked_text: String,
    #[serde(default)]
    hidden: bool,
    #[serde(default)]
    revealed_by: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FactFile {
    id: String,
    text: String,
    hidden: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CastFile {
    schema: String,
    id: String,
    kind: String,
    display_name: String,
    facts: Vec<FactFile>,
    #[serde(default)]
    tick: Option<TickProfile>,
    #[serde(default)]
    rules: Vec<RuleEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ItemFile {
    schema: String,
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    rules: Vec<RuleEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoryFile {
    schema: String,
    id: String,
    #[serde(default)]
    rules: Vec<RuleEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MetersFile {
    schema: String,
    meters: Vec<MeterEntryFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MeterEntryFile {
    id: String,
    min: i64,
    base_cap: i64,
    max: i64,
    start: i64,
    #[serde(default)]
    hidden: bool,
}

#[cfg(test)]
mod tests {
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

    #[test]
    fn load_meters_rejects_a_broken_inequality_a_duplicate_id_and_an_unknown_key() {
        let broken = "\
schema: meters
meters:
  - id: grit
    min: 2
    base_cap: 1
    max: 3
    start: 1
";
        let err = load_meters(broken).unwrap_err();
        assert!(
            err.iter()
                .any(|diag| diag.message.contains("min above base_cap")),
            "{err:?}"
        );
        assert_eq!(reason_of(err), Reason::Schema);
        let duplicate = "\
schema: meters
meters:
  - id: grit
    min: 0
    base_cap: 1
    max: 1
    start: 1
  - id: grit
    min: 0
    base_cap: 1
    max: 1
    start: 0
";
        let err = load_meters(duplicate).unwrap_err();
        assert!(
            err.iter().any(|diag| diag.message.contains("listed twice")),
            "{err:?}"
        );
        assert_eq!(reason_of(err), Reason::Schema);
        let unknown = "\
schema: meters
meters:
  - id: grit
    min: 0
    base_cap: 1
    max: 1
    start: 1
    kind: stat
";
        let err = load_meters(unknown).unwrap_err();
        assert_eq!(err.len(), 1, "{err:?}");
        assert!(err[0].message.contains("unknown field 'kind'"), "{err:?}");
        assert_eq!(reason_of(err), Reason::Schema);
    }

    fn rejects(load: fn(&str) -> Result<(), Vec<Diagnostic>>, good: &str, old: &str, body: &str) {
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
    fn load_place_rejects_an_unknown_tag_and_the_old_versioned_tag() {
        rejects(
            |text| load_place("world/places/gate.md", text, 0).map(|_| ()),
            "place",
            concat!("place/", "v0"),
            "id: gate\ndisplay_name: Gate\nfacts: []\n",
        );
    }

    #[test]
    fn load_cast_rejects_an_unknown_tag_and_the_old_versioned_tag() {
        rejects(
            |text| load_cast("cast/mira.md", text, 0).map(|_| ()),
            "cast",
            concat!("cast/", "v0"),
            "id: mira\nkind: npc\ndisplay_name: Mira\nfacts: []\n",
        );
    }

    #[test]
    fn load_pack_rejects_an_unknown_tag_and_the_old_versioned_tag() {
        rejects(
            |text| load_pack(text).map(|_| ()),
            "pack",
            concat!("pack/", "v0"),
            "id: ex\ntitle: Ex\nsystem: ex\nstart:\n  place: gate\n  pc: pc_rowan\n",
        );
    }

    #[test]
    fn load_meters_rejects_an_unknown_tag_and_the_old_versioned_tag() {
        rejects(
            |text| load_meters(text).map(|_| ()),
            "meters",
            concat!("meters/", "v0"),
            "meters: []\n",
        );
    }
}
