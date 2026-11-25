//! `world/map.yaml`: every path between places, each path as an exit from a place it leaves,
//! and the index of each place's exits.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

use crate::canon::rule_list;
use crate::canon::source::{load_tagged, Diags, FileText, MAP_SCHEMA};
use crate::canon::Canon;
use crate::diagnostic::{schema_error, Diagnostic};
use crate::ecs::component::ActorLocation;
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
    let Some((end_place_ids, bidirectional)) = split_ends(&line.path) else {
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
    let exit_id = line.id.as_deref().unwrap_or(end_place_ids[1].as_str());
    let path = PathDef {
        id: PathId::from(format!("{}.{exit_id}", end_place_ids[0])),
        end_place_ids,
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
    let mut exit_def_indexes_by_place_id: BTreeMap<PlaceId, Vec<usize>> = BTreeMap::new();
    let mut taken = BTreeSet::new();
    let mut diags = Diags::default();
    for (row, path) in paths.iter().enumerate() {
        let from_place_ids =
            (path.end_place_ids.iter()).take(if path.bidirectional { 2 } else { 1 });
        for exit in
            from_place_ids.filter_map(|from_place_id| path.exit_from(from_place_id.as_str()))
        {
            if taken.insert((exit.from_place_id, exit.id())) {
                let indexes = exit_def_indexes_by_place_id.entry(exit.from_place_id.clone());
                indexes.or_default().push(row);
                continue;
            }
            let problem = format!(
                "exit '{}' is listed twice at {}",
                exit.id(),
                exit.from_place_id
            );
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
    diags.into_result(exit_def_indexes_by_place_id)
}

/// The one flag a path with an `if` carries, set once someone at either end meets the `if`.
pub(crate) const OPEN: &str = "open";

/// What the pack says about one path.
#[derive(Clone, Debug)]
pub struct PathDef {
    pub id: PathId,
    /// The two places the line names, in its order.
    pub end_place_ids: [PlaceId; 2],
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
    /// The path walked toward `to_place_id`: from its first end, or from its second when it
    /// goes both ways. `None` when it does not lead there.
    pub(crate) fn exit_to(&self, to_place_id: &str) -> Option<Exit<'_>> {
        let [first, second] = &self.end_place_ids;
        let from_place_id = if second == to_place_id { first } else { second };
        self.exit_from(from_place_id.as_str())
            .filter(|exit| exit.to_place_id() == to_place_id)
    }

    /// This path as an exit from `from_place_id`: from its first end, or from its second when it
    /// goes both ways. `None` when it does not leave `from_place_id`.
    pub fn exit_from(&self, from_place_id: &str) -> Option<Exit<'_>> {
        let [first, second] = &self.end_place_ids;
        let from_place_id = if first == from_place_id {
            first
        } else if self.bidirectional && second == from_place_id {
            second
        } else {
            return None;
        };
        Some(Exit {
            path_def: self,
            from_place_id,
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

    /// Whether the path has flag `key`: only `open`, and only on a path with an `if`.
    pub(crate) fn has_flag(&self, key: &str) -> bool {
        key == OPEN && self.is_conditional()
    }
}

/// A path as seen from a place it leaves, named by the word a player types there.
#[derive(Clone, Copy, Debug)]
pub struct Exit<'c> {
    pub path_def: &'c PathDef,
    pub from_place_id: &'c PlaceId,
}

impl<'c> Exit<'c> {
    /// The word a player types here: the pack's `id`, or else the place it goes to.
    pub(crate) fn id(self) -> &'c str {
        let to_place_id = self.to_place_id().as_str();
        self.path_def.exit_id.as_deref().unwrap_or(to_place_id)
    }

    /// Where it goes: the end of its path that it does not leave from.
    pub(crate) fn to_place_id(self) -> &'c PlaceId {
        let [first, second] = &self.path_def.end_place_ids;
        if first == self.from_place_id {
            second
        } else {
            first
        }
    }

    /// Where someone walking this exit stands: on its path, heading where it leads.
    pub(crate) fn walker_location(&self) -> ActorLocation {
        ActorLocation::Path {
            path_id: self.path_def.id.clone(),
            to_place_id: self.to_place_id().clone(),
        }
    }

    /// The name a player reads: the name of the place it goes to when `id` is that place's id,
    /// or else `id`.
    pub(crate) fn name(&self, canon: &'c Canon) -> &'c str {
        let to_place_id = self.to_place_id();
        match canon.places.get(to_place_id) {
            Some(place) if to_place_id == self.id() => &place.name,
            _ => self.id(),
        }
    }
}

/// The pack's paths, and the index of each place's exits and of each path's id.
#[derive(Clone, Debug)]
pub struct Paths {
    path_defs: Vec<PathDef>,
    /// Each place's exits, as indexes into `path_defs` in file order.
    exit_def_indexes_by_place_id: BTreeMap<PlaceId, Vec<usize>>,
    def_index_by_path_id: BTreeMap<PathId, usize>,
}

impl Paths {
    /// Index each place's exits and each path's id from `path_defs`, in file order. Two paths
    /// never share an id: an id is an exit's place and word, which `index_exits` keeps unique.
    pub(super) fn new(path_defs: Vec<PathDef>) -> Result<Self, Vec<Diagnostic>> {
        let exit_def_indexes_by_place_id = index_exits(&path_defs)?;
        let def_indexes = path_defs.iter().enumerate();
        let def_index_by_path_id =
            (def_indexes.map(|(index, path_def)| (path_def.id.clone(), index))).collect();
        Ok(Self {
            path_defs,
            exit_def_indexes_by_place_id,
            def_index_by_path_id,
        })
    }

    pub(crate) fn get(&self, id: &PathId) -> Option<&PathDef> {
        (self.def_index_by_path_id.get(id)).map(|&index| &self.path_defs[index])
    }

    /// Every path's id, in id order.
    pub(crate) fn ids(&self) -> impl Iterator<Item = &PathId> {
        self.def_index_by_path_id.keys()
    }

    /// The exits of `from_place_id`, in the file order of their paths.
    pub(crate) fn exits(&self, from_place_id: &str) -> impl Iterator<Item = Exit<'_>> {
        let indexes = self
            .exit_def_indexes_by_place_id
            .get_key_value(from_place_id);
        indexes.into_iter().flat_map(|(from_place_id, indexes)| {
            (indexes.iter())
                .filter_map(|&index| self.path_defs[index].exit_from(from_place_id.as_str()))
        })
    }

    /// The exit at `from_place_id` that a player names `id`.
    pub(crate) fn find_exit(&self, from_place_id: &str, id: &str) -> Option<Exit<'_>> {
        self.exits(from_place_id).find(|exit| exit.id() == id)
    }

    /// Each path, in file order.
    pub fn iter(&self) -> impl Iterator<Item = &PathDef> {
        self.path_defs.iter()
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
