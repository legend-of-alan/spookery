//! `world/map.yaml`: every path between places, each path as an exit from a place it leaves,
//! and the index of each place's exits.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

use crate::canon::rule_list;
use crate::canon::source::{load_tagged, Diags, FileText, MAP_SCHEMA};
use crate::canon::Canon;
use crate::diagnostic::{schema_error, Diagnostic};
use crate::ecs::component::Location;
use crate::ecs::id::{PathId, PlaceId};
use crate::rules::expr::Expr;
use crate::rules::rule::{self, Cond, Cost, TargetKind};

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
    let exit_id = line.id.as_deref().unwrap_or(ends[1].as_str());
    let path = PathDef {
        id: PathId::from(format!("{}.{exit_id}", ends[0])),
        ends,
        bidirectional,
        exit_id: line.id.clone(),
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
            let field = if path.exit_id.is_some() {
                ".id"
            } else {
                ".path"
            };
            diags.push(
                schema_error(MAP_FILE, &problem, hint).at_path(format!("paths[{row}]{field}")),
            );
        }
    }
    diags.into_result(exit_rows)
}

/// The one flag a path with an `if` carries, set once someone at either end meets the `if`.
pub(crate) const OPEN: &str = "open";

/// What the pack says about one path.
#[derive(Clone, Debug)]
pub struct PathDef {
    pub id: PathId,
    /// The two places the line names, in its order.
    pub ends: [PlaceId; 2],
    /// The line says `<->`.
    pub bidirectional: bool,
    /// The word a player types at each end, the pack's `id`. Left out, the other end's place id.
    pub exit_id: Option<String>,
    /// `cost.ticks` is the game time the walk takes.
    pub cost: Cost,
    pub if_: Vec<Cond>,
    pub locked_text: String,
    /// The flag that shows the exit, to an actor who carries it or to everyone once the world
    /// does. An exit without one is always shown.
    pub revealed_by: Option<String>,
}

impl PathDef {
    /// The path walked toward `to_id`: from its first end, or from its second when it goes both
    /// ways. `None` when it does not lead there.
    pub(crate) fn exit_to(&self, to_id: &str) -> Option<Exit<'_>> {
        let [first, second] = &self.ends;
        let from = if second == to_id { first } else { second };
        self.exit_from(from.as_str())
            .filter(|exit| exit.to == to_id)
    }

    /// This path as an exit from `place`: from its first end, or from its second when it goes
    /// both ways. `None` when it does not leave `place`.
    pub fn exit_from(&self, place: &str) -> Option<Exit<'_>> {
        let [first, second] = &self.ends;
        let (from, to) = if first == place {
            (first, second)
        } else if self.bidirectional && second == place {
            (second, first)
        } else {
            return None;
        };
        let id = self.exit_id.as_deref().unwrap_or(to.as_str());
        Some(Exit {
            id,
            from,
            to,
            path: self,
        })
    }

    /// Every number the path holds: its cost's, then its conditions'.
    pub(crate) fn exprs(&self) -> Vec<&Expr> {
        let cond_numbers = self.if_.iter().flat_map(Cond::exprs);
        self.cost.exprs().chain(cond_numbers).collect()
    }

    /// The owners, item ids and `target`s the path's conditions, cost and numbers name.
    pub(crate) fn mentions(&self) -> rule::Mentions<'_> {
        rule::Mentions::of_lines(&self.if_, [&self.cost], self.exprs())
    }

    /// The path has an `if`, so the exit can stay locked until it opens.
    pub(crate) fn is_conditional(&self) -> bool {
        !self.if_.is_empty()
    }

    /// The name a player reads for the path: its exit's from its first end.
    pub(crate) fn name<'c>(&'c self, canon: &'c Canon) -> &'c str {
        let [first, _] = &self.ends;
        self.exit_from(first.as_str())
            .map_or(first.as_str(), |exit| exit.name(canon))
    }

    /// Whether the path carries flag `key`: only `open`, and only on a path with an `if`.
    pub(crate) fn carries_flag(&self, key: &str) -> bool {
        key == OPEN && self.is_conditional()
    }
}

/// A path as seen from a place it leaves, named by the word a player types there.
#[derive(Clone, Copy, Debug)]
pub struct Exit<'c> {
    pub id: &'c str,
    pub from: &'c PlaceId,
    pub to: &'c PlaceId,
    pub path: &'c PathDef,
}

impl<'c> Exit<'c> {
    /// Where someone walking this exit stands: on its path, heading where it leads.
    pub(crate) fn walker_location(&self) -> Location {
        Location::Path {
            path: self.path.id.clone(),
            to: self.to.clone(),
        }
    }

    /// The name a player reads: the name of the place it goes to when `id` is that place's id,
    /// or else `id`.
    pub(crate) fn name(&self, canon: &'c Canon) -> &'c str {
        match canon.places.get(self.to) {
            Some(place) if self.to == self.id => &place.name,
            _ => self.id,
        }
    }
}

/// The pack's paths, and the index of each place's exits and of each path's id.
#[derive(Clone, Debug)]
pub struct Paths {
    paths: Vec<PathDef>,
    exit_rows: BTreeMap<PlaceId, Vec<usize>>,
    rows_by_id: BTreeMap<PathId, usize>,
}

impl Paths {
    /// Index each place's exits and each path's id from `paths`, in file order. Two paths
    /// never share an id: an id is an exit's place and word, which `index_exits` keeps unique.
    pub(super) fn index(paths: Vec<PathDef>) -> Result<Self, Vec<Diagnostic>> {
        let exit_rows = index_exits(&paths)?;
        let rows = paths.iter().enumerate();
        let rows_by_id = rows.map(|(row, path)| (path.id.clone(), row)).collect();
        Ok(Self {
            paths,
            exit_rows,
            rows_by_id,
        })
    }

    pub(crate) fn get(&self, id: &PathId) -> Option<&PathDef> {
        self.rows_by_id.get(id).map(|&row| &self.paths[row])
    }

    /// Every path's id, in id order.
    pub(crate) fn ids(&self) -> impl Iterator<Item = &PathId> {
        self.rows_by_id.keys()
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
