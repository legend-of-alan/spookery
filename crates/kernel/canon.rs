use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::check::{load_checks, CheckDef};
use crate::diagnostic::{
    coded, locate_nth, locate_row_field, parse_file, schema, tag_diagnostic, with_spot, Diagnostic,
};
use crate::error::Reason;
use crate::hash::blake2s_hex;
use crate::limits::{load_tick_rules, Budget};
use crate::meter::{self, MeterDef, MeterLive};
use crate::model::Now;
use crate::moves::{load_moves, Audience, MoveDef, RowEffect};

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
    pub moves: BTreeMap<String, MoveDef>,
    pub checks: BTreeMap<String, CheckDef>,
    pub meters: BTreeMap<String, MeterDef>,
    pub budget: Budget,
}

#[derive(Clone, Debug)]
pub struct FactCanon {
    pub text: String,
    pub hidden: bool,
    pub owner_id: String,
}

#[derive(Clone, Debug)]
pub struct PlaceCanon {
    pub facts: Vec<String>,
    pub body: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CastKind {
    Npc,
    PcTemplate,
}

#[derive(Clone, Debug)]
pub struct CastCanon {
    pub kind: CastKind,
    pub facts: Vec<String>,
    pub body: String,
    pub tick: Option<TickProfile>,
}

#[derive(Clone, Debug)]
pub struct TickProfile {
    pub eligible: bool,
    pub range: Vec<String>,
    pub stock_rate: BTreeMap<String, BTreeMap<String, i64>>,
    pub flags: Vec<String>,
    pub habits: Vec<Habit>,
}

#[derive(Clone, Debug)]
pub struct Habit {
    pub id: String,
    pub runs_while_empty: bool,
    pub from_hour: Option<i64>,
    pub to_hour: Option<i64>,
    pub clock: Option<String>,
    pub at_least: Option<i64>,
    pub unless_message: Option<String>,
    pub rows: Vec<String>,
}

impl Canon {
    pub fn profile(&self, actor_id: &str) -> Option<&TickProfile> {
        self.cast.get(actor_id).and_then(|cast| cast.tick.as_ref())
    }

    pub fn body_of_fact(&self, fact_id: &str) -> &str {
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

    pub fn needs_roll(&self, move_name: &str) -> bool {
        self.moves
            .get(move_name)
            .is_some_and(|def| def.check.is_some())
    }

    /// Difficulty on a row. A move without a row uses 0.
    pub fn row_difficulty(&self, move_name: &str) -> i64 {
        self.moves
            .get(move_name)
            .and_then(|def| def.row.as_ref())
            .map(|row| row.difficulty)
            .unwrap_or(0)
    }

    /// Lower band on a row. `None` keeps that roll binary.
    pub fn row_partial_at(&self, move_name: &str) -> Option<i64> {
        self.moves
            .get(move_name)
            .and_then(|def| def.row.as_ref())
            .and_then(|row| row.partial_at)
    }

    /// Check id named by a row. A move without `effects` is not a row.
    pub fn row_check(&self, move_name: &str) -> Option<String> {
        let def = self.moves.get(move_name)?;
        def.row.as_ref()?;
        def.check.clone()
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
    let mut place_files = Vec::new();
    if let Err(reason) = collect_files(&root.join("world"), root, &mut place_files) {
        diags.push(io_problem("world", reason, "could not read world/"));
    }
    for (rel, bytes) in place_files {
        if !rel.ends_with(".md") {
            continue;
        }
        let Ok(text) = std::str::from_utf8(&bytes) else {
            diags.push(
                schema(&rel, "this file is not UTF-8 text", "save it as UTF-8").with_path("schema"),
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
        if places.contains_key(&file.id) {
            diags.push(
                schema(
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
                facts: ids,
                body: body.to_string(),
            },
        );
    }
    let mut cast = BTreeMap::new();
    let mut cast_files_by_id = BTreeMap::new();
    let mut cast_files = Vec::new();
    if let Err(reason) = collect_files(&root.join("cast"), root, &mut cast_files) {
        diags.push(io_problem("cast", reason, "could not read cast/"));
    }
    for (rel, bytes) in cast_files {
        if !rel.ends_with(".md") {
            continue;
        }
        let Ok(text) = std::str::from_utf8(&bytes) else {
            diags.push(
                schema(&rel, "this file is not UTF-8 text", "save it as UTF-8").with_path("schema"),
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
        let (tick, tick_ok) = match file.tick {
            Some(tick) => match validate_tick(&rel, text, tick) {
                Ok(profile) => (Some(profile), true),
                Err(more) => {
                    diags.extend(more);
                    (None, false)
                }
            },
            None => (None, true),
        };
        if !fact_ok || !tick_ok {
            continue;
        }
        if let Some(profile) = &tick {
            for (index, place_id) in profile.range.iter().enumerate() {
                if places.contains_key(place_id) {
                    continue;
                }
                let mut diag = schema(
                    &rel,
                    &format!("range names '{place_id}', which is not a place"),
                    "use a place id from world/",
                )
                .with_path(format!("tick.range[{index}]"));
                if let Some((line, column)) = locate_nth(text, place_id, 0) {
                    diag = diag.at_line(line, Some(column));
                }
                diags.push(diag);
            }
        }
        let kind = match file.kind.as_str() {
            "npc" => CastKind::Npc,
            "pc_template" => CastKind::PcTemplate,
            _ => {
                diags.push(headed_problem(
                    &rel,
                    yaml,
                    1,
                    "kind:",
                    "kind",
                    &format!("kind is '{}', which is not npc or pc_template", file.kind),
                    "set kind to npc or pc_template",
                ));
                continue;
            }
        };
        if cast.contains_key(&file.id) {
            diags.push(
                schema(
                    &rel,
                    &format!("cast id '{}' is already defined", file.id),
                    "keep one cast file with this id",
                )
                .with_path("id"),
            );
            continue;
        }
        let id = file.id.clone();
        cast_files_by_id.insert(id.clone(), rel);
        cast.insert(
            id.clone(),
            CastCanon {
                kind,
                facts: fact_ids_of(&facts, &id),
                body: body.to_string(),
                tick,
            },
        );
    }
    let moves_text = match read_text(root, "rules/moves.yaml") {
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
    let moves = match &moves_text {
        Some(text) => match load_moves(text) {
            Ok(moves) => Some(moves),
            Err(more) => {
                diags.extend(more);
                None
            }
        },
        None => None,
    };
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
        Some(text) => match load_tick_rules(text) {
            Ok(budget) => Some(budget),
            Err(more) => {
                diags.extend(more);
                None
            }
        },
        None => None,
    };
    if let (Some(moves), Some(checks)) = (&moves, &checks) {
        diags.extend(unknown_checks(moves, checks, moves_text.as_deref()));
    }
    if let (Some(meters), Some(checks), Some(moves)) = (&meters, &checks, &moves) {
        diags.extend(unknown_meters(meters, checks, moves));
    }
    if let Some(moves) = &moves {
        diags.extend(habit_row_problems(&cast, moves, &cast_files_by_id));
    }
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
        moves: moves.expect("moves loaded"),
        checks: checks.expect("checks loaded"),
        meters: meters.expect("meters loaded"),
        budget: budget.expect("tick loaded"),
    })
}

fn io_problem(file: &str, reason: Reason, problem: &str) -> Diagnostic {
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
        return Err(vec![fact_problem(
            file,
            text,
            index,
            "id:",
            "a fact id is empty",
            "set id to a short name",
        )]);
    }
    if facts.contains_key(&fact.id) {
        return Err(vec![fact_problem(
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

fn fact_problem(
    file: &str,
    text: &str,
    index: usize,
    field: &str,
    problem: &str,
    hint: &str,
) -> Diagnostic {
    let mut diag = schema(file, problem, hint).with_path(format!("facts[{index}].id"));
    if let Some((line, column)) = locate_row_field(text, "id:", index, field) {
        diag = diag.at_line(line, Some(column));
    }
    diag
}

fn validate_tick(file: &str, text: &str, tick: TickFile) -> Result<TickProfile, Vec<Diagnostic>> {
    let habits = habits_of(file, text, tick.habits)?;
    Ok(TickProfile {
        eligible: tick.eligible,
        range: tick.range,
        stock_rate: tick.stock_rate,
        flags: tick.flags,
        habits,
    })
}

fn habits_of(
    file: &str,
    text: &str,
    entries: Vec<HabitFile>,
) -> Result<Vec<Habit>, Vec<Diagnostic>> {
    let mut habits = Vec::with_capacity(entries.len());
    let mut diags = Vec::new();
    let mut seen = BTreeSet::new();
    for (index, entry) in entries.into_iter().enumerate() {
        let path = format!("tick.habits[{index}]");
        let mut bad = false;
        if entry.id.is_empty() {
            diags.push(habit_problem(
                file,
                text,
                &path,
                "id:",
                "a habit id is empty",
                "set id to a short name",
            ));
            bad = true;
        } else if !seen.insert(entry.id.clone()) {
            diags.push(habit_problem(
                file,
                text,
                &format!("{path}.id"),
                "id:",
                &format!("habit '{}' is listed twice", entry.id),
                "keep one habit",
            ));
            bad = true;
        }
        if entry.rows.is_empty() {
            diags.push(habit_problem(
                file,
                text,
                &format!("{path}.rows"),
                "rows:",
                &format!("habit '{}' has no rows", entry.id),
                "name at least one move",
            ));
            bad = true;
        }
        if entry.rows.iter().any(|row| row.is_empty()) {
            diags.push(habit_problem(
                file,
                text,
                &format!("{path}.rows"),
                "rows:",
                &format!("habit '{}' names an empty row", entry.id),
                "use a move name",
            ));
            bad = true;
        }
        let hours = match (entry.from_hour, entry.to_hour) {
            (None, None) => (None, None),
            (Some(from), Some(to)) if (0..24).contains(&from) && (0..24).contains(&to) => {
                (Some(from), Some(to))
            }
            _ => {
                diags.push(habit_problem(
                    file,
                    text,
                    &format!("{path}.from_hour"),
                    "from_hour:",
                    &format!(
                        "habit '{}' has a half-set or out-of-range hour window",
                        entry.id
                    ),
                    "set both from_hour and to_hour to integers in 0..24, or omit both",
                ));
                bad = true;
                (None, None)
            }
        };
        let clock = match (entry.clock.clone(), entry.at_least) {
            (None, None) => (None, None),
            (Some(id), Some(at_least)) if !id.is_empty() => (Some(id), Some(at_least)),
            _ => {
                diags.push(habit_problem(
                    file,
                    text,
                    &format!("{path}.clock"),
                    "clock:",
                    &format!("habit '{}' has a half-set clock gate", entry.id),
                    "set both clock and at_least, or omit both",
                ));
                bad = true;
                (None, None)
            }
        };
        if entry
            .unless_message
            .as_ref()
            .is_some_and(|id| id.is_empty())
        {
            diags.push(habit_problem(
                file,
                text,
                &format!("{path}.unless_message"),
                "unless_message:",
                &format!("habit '{}' has an empty unless_message", entry.id),
                "name a message id or omit unless_message",
            ));
            bad = true;
        }
        if bad {
            continue;
        }
        habits.push(Habit {
            id: entry.id,
            runs_while_empty: entry.runs_while_empty,
            from_hour: hours.0,
            to_hour: hours.1,
            clock: clock.0,
            at_least: clock.1,
            unless_message: entry.unless_message,
            rows: entry.rows,
        });
    }
    if diags.is_empty() {
        Ok(habits)
    } else {
        Err(diags)
    }
}

fn habit_problem(
    file: &str,
    text: &str,
    path: &str,
    field: &str,
    problem: &str,
    hint: &str,
) -> Diagnostic {
    let mut diag = schema(file, problem, hint).with_path(path);
    if let Some((line, column)) = locate_nth(text, field, 0) {
        diag = diag.at_line(line, Some(column));
    }
    diag
}

pub fn load_meters(text: &str) -> Result<BTreeMap<String, MeterDef>, Vec<Diagnostic>> {
    const FILE: &str = "rules/meters.yaml";
    if let Some(diag) = tag_diagnostic(text, crate::format::METERS, FILE, 0) {
        return Err(vec![diag]);
    }
    let file: MetersFile = parse_file(FILE, text, 0)?;
    let mut diags = Vec::new();
    if file.schema != crate::format::METERS {
        diags.push(
            schema(
                FILE,
                &format!("schema is '{}', which this file does not use", file.schema),
                "write 'schema: meters' with no version suffix",
            )
            .with_path("schema"),
        );
    }
    let mut meters = BTreeMap::new();
    for (index, entry) in file.meters.into_iter().enumerate() {
        let path = format!("meters[{index}]");
        let mut bad = false;
        if entry.id.is_empty() {
            diags.push(meter_problem(
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
            diags.push(meter_problem(
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
            diags.push(meter_problem(
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
            diags.push(meter_problem(
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
            diags.push(meter_problem(
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

fn meter_problem(
    text: &str,
    index: usize,
    field: &str,
    path: &str,
    problem: &str,
    hint: &str,
) -> Diagnostic {
    let mut diag = schema("rules/meters.yaml", problem, hint).with_path(path);
    if let Some((line, column)) = locate_row_field(text, "id:", index, field) {
        diag = diag.at_line(line, Some(column));
    }
    diag
}

/// Unknown ids and pairs `stored` rejects are `SCHEMA`. A defined id missing
/// from a present map is filled from `start` with offset 0.
pub fn bind_meters(canon: &Canon, now: &mut Now) -> Result<(), Vec<Diagnostic>> {
    let mut diags = Vec::new();
    for (id, actor) in &now.actors {
        for key in actor.meters().keys() {
            if canon.meters.contains_key(key) {
                continue;
            }
            diags.push(
                schema(
                    "state/now.yaml",
                    &format!(
                        "actor '{id}' lists meter '{key}', which rules/meters.yaml does not define"
                    ),
                    "remove the meter or add it to the rules",
                )
                .with_path(format!("actors.{id}.meters.{key}")),
            );
        }
        for (key, meter) in actor.meters() {
            let Some(def) = canon.meters.get(key) else {
                continue;
            };
            if meter::stored(def, meter.base, meter.offset) {
                continue;
            }
            diags.push(
                schema(
                    "state/now.yaml",
                    &format!("actor '{id}' meter '{key}' is outside the rules"),
                    "set base and offset so the stored pair fits the meter",
                )
                .with_path(format!("actors.{id}.meters.{key}")),
            );
        }
    }
    if !diags.is_empty() {
        return Err(diags);
    }
    for actor in now.actors.values_mut() {
        let live = actor.meters_mut();
        for (id, def) in &canon.meters {
            live.entry(id.clone())
                .or_insert_with(|| MeterLive::at_start(def));
        }
    }
    Ok(())
}

/// An empty id or a duplicate is `SCHEMA`. This does not fill and does not
/// look at meter pairs.
pub fn bind_inventory(now: &Now) -> Result<(), Vec<Diagnostic>> {
    let mut diags = Vec::new();
    for (id, actor) in &now.actors {
        let mut seen = BTreeSet::new();
        for item in actor.inventory() {
            if item.is_empty() {
                diags.push(
                    schema(
                        "state/now.yaml",
                        &format!("actor '{id}' lists an empty item id"),
                        "give the item a non-empty id",
                    )
                    .with_path(format!("actors.{id}.inventory")),
                );
            } else if !seen.insert(item.as_str()) {
                diags.push(
                    schema(
                        "state/now.yaml",
                        &format!("actor '{id}' lists item '{item}' twice"),
                        "keep one of that item",
                    )
                    .with_path(format!("actors.{id}.inventory")),
                );
            }
        }
    }
    if diags.is_empty() {
        Ok(())
    } else {
        Err(diags)
    }
}

fn unknown_checks(
    moves: &BTreeMap<String, MoveDef>,
    checks: &BTreeMap<String, CheckDef>,
    text: Option<&str>,
) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    for (name, def) in moves {
        let Some(id) = &def.check else { continue };
        if checks.contains_key(id) {
            continue;
        }
        let mut diag = schema(
            "rules/moves.yaml",
            &format!("move '{name}' names check '{id}', which is not a check in rules/checks.yaml"),
            "add the check or name one that exists",
        )
        .with_path(format!("moves.{name}.check"));
        if let Some(text) = text {
            if let Some((line, column)) = locate_nth(text, &format!("check: {id}"), 0) {
                diag = diag.at_line(line, Some(column));
            }
        }
        diags.push(diag);
    }
    diags
}

fn unknown_meters(
    meters: &BTreeMap<String, MeterDef>,
    checks: &BTreeMap<String, CheckDef>,
    moves: &BTreeMap<String, MoveDef>,
) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    for (id, check) in checks {
        if meters.contains_key(&check.stat) {
            continue;
        }
        diags.push(
            schema(
                "rules/checks.yaml",
                &format!(
                    "check '{id}' uses stat '{}', which is not in rules/meters.yaml",
                    check.stat
                ),
                "add the meter or name one that exists",
            )
            .with_path(format!("checks.{id}.stat")),
        );
    }
    for (name, def) in moves {
        let Some(row) = def.row.as_ref() else {
            continue;
        };
        for effect in row
            .effects
            .iter()
            .chain(&row.success)
            .chain(&row.partial)
            .chain(&row.failure)
        {
            let RowEffect::Meter(write) = effect else {
                continue;
            };
            if meters.contains_key(&write.key) {
                continue;
            }
            diags.push(
                schema(
                    "rules/moves.yaml",
                    &format!(
                        "move '{name}' writes meter '{}', which is not in rules/meters.yaml",
                        write.key
                    ),
                    "add the meter or name one that exists",
                )
                .with_path(format!("moves.{name}.effects")),
            );
        }
    }
    diags
}

fn habit_row_problems(
    cast: &BTreeMap<String, CastCanon>,
    moves: &BTreeMap<String, MoveDef>,
    files: &BTreeMap<String, String>,
) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    for (id, profile) in cast {
        let Some(tick) = profile.tick.as_ref() else {
            continue;
        };
        let file = files.get(id).map(String::as_str).unwrap_or("cast");
        for habit in &tick.habits {
            for name in &habit.rows {
                let Some(row) = moves.get(name).and_then(|def| def.row.as_ref()) else {
                    diags.push(
                        schema(
                            file,
                            &format!(
                                "habit '{}' names '{name}', which is not a move row",
                                habit.id
                            ),
                            "add the move or fix the name",
                        )
                        .with_path(format!("tick.habits.{}.rows", habit.id)),
                    );
                    continue;
                };
                if row.audience != Some(Audience::Habit) {
                    diags.push(
                        schema(
                            file,
                            &format!(
                                "habit '{}' names '{name}', which is not an audience: habit row",
                                habit.id
                            ),
                            "point it at a habit row",
                        )
                        .with_path(format!("tick.habits.{}.rows", habit.id)),
                    );
                }
            }
        }
    }
    diags
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
    load_headed(
        "pack.yaml",
        text,
        crate::format::PACK,
        0,
        |file: PackFile| {
            let mut diags = Vec::new();
            if file.schema != crate::format::PACK {
                diags.push(
                    schema(
                        "pack.yaml",
                        &format!("schema is '{}', which this file does not use", file.schema),
                        "write 'schema: pack' with no version suffix",
                    )
                    .with_path("schema"),
                );
            }
            if file.id.is_empty() {
                diags.push(
                    schema("pack.yaml", "id is empty", "set id to a short pack id").with_path("id"),
                );
            }
            if diags.is_empty() {
                Ok(file)
            } else {
                Err(diags)
            }
        },
    )
}

fn load_place(file: &str, yaml: &str, line_base: u32) -> Result<PlaceFile, Vec<Diagnostic>> {
    load_headed(
        file,
        yaml,
        crate::format::PLACE,
        line_base,
        |parsed: PlaceFile| {
            let mut diags = Vec::new();
            if parsed.schema != crate::format::PLACE {
                diags.push(headed_problem(
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
                diags.push(headed_problem(
                    file,
                    yaml,
                    line_base,
                    "id:",
                    "id",
                    "id is empty",
                    "set id to a short place id",
                ));
            }
            if diags.is_empty() {
                Ok(parsed)
            } else {
                Err(diags)
            }
        },
    )
}

fn load_cast(file: &str, yaml: &str, line_base: u32) -> Result<CastFile, Vec<Diagnostic>> {
    load_headed(
        file,
        yaml,
        crate::format::CAST,
        line_base,
        |parsed: CastFile| {
            let mut diags = Vec::new();
            if parsed.schema != crate::format::CAST {
                diags.push(headed_problem(
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
                diags.push(headed_problem(
                    file,
                    yaml,
                    line_base,
                    "id:",
                    "id",
                    "id is empty",
                    "set id to a short cast id",
                ));
            }
            if parsed.kind != "npc" && parsed.kind != "pc_template" {
                diags.push(headed_problem(
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
        },
    )
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

fn headed_problem(
    file: &str,
    text: &str,
    line_base: u32,
    field: &str,
    path: &str,
    problem: &str,
    hint: &str,
) -> Diagnostic {
    with_spot(
        schema(file, problem, hint).with_path(path),
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
            return Err(vec![schema(
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
    Err(vec![schema(
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
    facts: Vec<FactFile>,
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
    facts: Vec<FactFile>,
    #[serde(default)]
    tick: Option<TickFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TickFile {
    eligible: bool,
    #[serde(default)]
    range: Vec<String>,
    #[serde(default)]
    stock_rate: BTreeMap<String, BTreeMap<String, i64>>,
    #[serde(default)]
    flags: Vec<String>,
    #[serde(default)]
    habits: Vec<HabitFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HabitFile {
    id: String,
    runs_while_empty: bool,
    #[serde(default)]
    from_hour: Option<i64>,
    #[serde(default)]
    to_hour: Option<i64>,
    #[serde(default)]
    clock: Option<String>,
    #[serde(default)]
    at_least: Option<i64>,
    #[serde(default)]
    unless_message: Option<String>,
    rows: Vec<String>,
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
        let text = fs::read_to_string(root.join("state/now.yaml")).unwrap();
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
            "id: gate\nfacts: []\n",
        );
    }

    #[test]
    fn load_cast_rejects_an_unknown_tag_and_the_old_versioned_tag() {
        rejects(
            |text| load_cast("cast/mira.md", text, 0).map(|_| ()),
            "cast",
            concat!("cast/", "v0"),
            "id: mira\nkind: npc\nfacts: []\n",
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
