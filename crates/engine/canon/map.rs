//! `world/map.yaml`: every path between places, and the index of each place's exits.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

use crate::canon::rule_list;
use crate::canon::source::{load_tagged, Diags, FileText, MAP_SCHEMA};
use crate::canon::{Exit, PathDef};
use crate::diagnostic::{schema_error, Diagnostic};
use crate::ecs::id::PlaceId;
use crate::rules::expr::Expr;
use crate::rules::rule::{Cost, TargetKind};

/// The one pack file that lists the paths.
pub(super) const MAP_FILE: &str = "world/map.yaml";

/// Every line under `paths:`, in file order, or the refusals of the lines that do not compile.
pub(super) fn load(text: &str) -> Result<Vec<PathDef>, Vec<Diagnostic>> {
    let doc: MapDoc = load_tagged(MAP_FILE, text, MAP_SCHEMA)?;
    let mut diags = Diags::default();
    let paths = (doc.paths.iter().enumerate())
        .filter_map(|(row, line)| diags.keep(compile(row, line)))
        .collect();
    diags.into_result(paths)
}

/// The `row`th line under `paths:`: its ends, its word, its cost and its `if`.
fn compile(row: usize, line: &PathYaml) -> Result<PathDef, Vec<Diagnostic>> {
    let source = FileText::named(MAP_FILE);
    let refuse = |field: &str, problem: &str, hint: &str| {
        vec![schema_error(MAP_FILE, problem, hint).at_path(format!("paths[{row}]{field}"))]
    };
    let Some((ends, bidirectional)) = split_ends(&line.path) else {
        let problem = format!(
            "path '{}' needs two place ids joined by -> or <->",
            line.path
        );
        return Err(refuse(
            ".path",
            &problem,
            "write a -> b, or a <-> b for both ways",
        ));
    };
    if line.id.as_deref() == Some("") {
        let hint = "set id to the word a player types, or leave it out";
        return Err(refuse(".id", "a path id is empty", hint));
    }
    let cost = match &line.cost {
        Some(yaml) => rule_list::cost_of(yaml)
            .map_err(|fault| vec![source.fault(&format!("paths[{row}].cost"), fault)])?,
        None => Cost {
            ticks: Expr::Num(0),
            meters: Vec::new(),
            items: Vec::new(),
        },
    };
    let mut diags = Diags::default();
    let if_ = (line.if_.iter())
        .filter_map(|cond| {
            let fault = |fault| diags.push(source.fault(&format!("paths[{row}].if"), fault));
            rule_list::cond_of(cond).map_err(fault).ok()
        })
        .collect();
    diags.into_result(())?;
    let path = PathDef {
        ends,
        bidirectional,
        id: line.id.clone(),
        cost,
        if_,
        locked_text: line.locked_text.clone(),
        revealed_by: line.revealed_by.clone(),
    };
    if let Some((key, _)) = path.mentions().find_misplaced_target(TargetKind::None) {
        let problem = format!(
            "path '{}' says target in {key}, but an exit has no target",
            line.path
        );
        return Err(refuse(
            "",
            &problem,
            "an exit is walked, not targeted; name self or an id",
        ));
    }
    Ok(path)
}

/// `a -> b` or `a <-> b` as its two place ids and whether it goes both ways, or `None` for
/// any other line.
fn split_ends(line: &str) -> Option<([PlaceId; 2], bool)> {
    let (bidirectional, (first, second)) = match line.split_once("<->") {
        Some(ends) => (true, ends),
        None => (false, line.split_once("->")?),
    };
    let place = |end: &str| {
        let end = end.trim();
        (!end.is_empty() && !end.contains(char::is_whitespace)).then(|| PlaceId::from(end))
    };
    Some(([place(first)?, place(second)?], bidirectional))
}

/// Each place's exits as rows of `paths` in file order: a row goes under its first place, and
/// under its second too when it goes both ways. A word already taken at a place is refused.
pub(super) fn index_exits(
    paths: &[PathDef],
) -> Result<BTreeMap<PlaceId, Vec<usize>>, Vec<Diagnostic>> {
    let mut exit_rows: BTreeMap<PlaceId, Vec<usize>> = BTreeMap::new();
    let mut taken = BTreeSet::new();
    let mut diags = Diags::default();
    for (row, path) in paths.iter().enumerate() {
        let ends = path
            .ends
            .iter()
            .take(if path.bidirectional { 2 } else { 1 });
        for exit in ends.filter_map(|end| path.exit_from(end.as_str())) {
            if taken.insert((exit.from, exit.id)) {
                exit_rows.entry(exit.from.clone()).or_default().push(row);
                continue;
            }
            let problem = format!("exit '{}' is listed twice at {}", exit.id, exit.from);
            let hint = "give one of these paths its own id";
            let field = if path.id.is_some() { ".id" } else { ".path" };
            diags.push(
                schema_error(MAP_FILE, &problem, hint).at_path(format!("paths[{row}]{field}")),
            );
        }
    }
    diags.into_result(exit_rows)
}

/// The pack's paths, and the index of each place's exits.
#[derive(Clone, Debug)]
pub struct Paths {
    paths: Vec<PathDef>,
    exit_rows: BTreeMap<PlaceId, Vec<usize>>,
}

impl Paths {
    /// Index each place's exits from `paths`, in file order.
    pub(super) fn index(paths: Vec<PathDef>) -> Result<Self, Vec<Diagnostic>> {
        let exit_rows = index_exits(&paths)?;
        Ok(Self { paths, exit_rows })
    }

    /// The exits of `place`, in the file order of their paths.
    pub(crate) fn exits(&self, place: &str) -> impl Iterator<Item = Exit<'_>> {
        let place_rows = self.exit_rows.get_key_value(place).into_iter();
        place_rows.flat_map(|(from, rows)| {
            rows.iter()
                .filter_map(|&row| self.paths[row].exit_from(from.as_str()))
        })
    }

    /// The exit at `place` that a player names `id`.
    pub(crate) fn find_exit(&self, place: &str, id: &str) -> Option<Exit<'_>> {
        self.exits(place).find(|exit| exit.id == id)
    }

    /// Each path, in file order.
    pub fn iter(&self) -> impl Iterator<Item = &PathDef> {
        self.paths.iter()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MapDoc {
    // repeats module: every file shape in canon ends in Doc
    #[serde(rename = "schema")]
    _schema: String,
    paths: Vec<PathYaml>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PathYaml {
    path: String,
    id: Option<String>,
    cost: Option<rule_list::CostYaml>,
    #[serde(default, rename = "if")]
    if_: Vec<rule_list::CondYaml>,
    #[serde(default)]
    locked_text: String,
    revealed_by: Option<String>,
}
