//! `world/map.yaml`: every path between places, each path as an exit from a place it leaves,
//! and the index of each place's exits.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::canon::refusal::{Diagnostics, FileText};
use crate::canon::rule_list;
use crate::canon::Canon;
use crate::diagnostic::{parse_tagged, schema_error, Diagnostic};
use crate::ecs::component::ActorLocation;
use crate::ecs::id::{PathId, PlaceId};
use crate::rules::expr::Expr;
use crate::rules::mentions;
use crate::rules::rule::{Cond, Cost, TargetKind};

/// The one pack file that lists the paths.
pub(super) const FILE: &str = "world/map.yaml";
const SCHEMA: &str = "map";

/// Every entry under `paths:`, in file order, or the refusals of the entries that do not compile.
pub(super) fn load(text: &str) -> Result<Vec<PathDef>, Vec<Diagnostic>> {
    let yaml: Yaml = parse_tagged(FILE, text, SCHEMA, true)?;
    let mut diagnostics = Diagnostics::default();
    let path_defs = (yaml.paths.iter().enumerate())
        .filter_map(|(index, entry)| diagnostics.keep(compile(index, entry)))
        .collect();
    diagnostics.into_result(path_defs)
}

/// The `index`th entry under `paths:`: its ends, its id, its name, its cost and its `if`.
fn compile(index: usize, entry: &PathYaml) -> Result<PathDef, Vec<Diagnostic>> {
    let file_text = FileText::without_text(FILE);
    let refuse = |field: &str, message: &str, hint: &str| {
        vec![schema_error(FILE, message, hint).at_key_path(format!("paths[{index}]{field}"))]
    };
    let Some((end_place_ids, bidirectional)) = split_ends(&entry.path) else {
        let message = format!(
            "path '{}' needs two place ids joined by -> or <->",
            entry.path
        );
        return Err(refuse(
            ".path",
            &message,
            "write a -> b, or a <-> b for both ways",
        ));
    };
    let id = match entry.id.as_deref() {
        None => PathId::from(format!("{}.{}", end_place_ids[0], end_place_ids[1])),
        Some(id) if !id.is_empty() && !id.contains('.') => PathId::from(id),
        Some(id) => {
            let message = format!("path id '{id}' is not one word");
            let hint = "write the id as one word with no dot, or leave it out";
            return Err(refuse(".id", &message, hint));
        }
    };
    let cost = match &entry.cost {
        Some(yaml) => rule_list::compile::cost(yaml).map_err(|error| {
            vec![file_text.refuse_error(&format!("paths[{index}].cost"), error)]
        })?,
        None => Cost {
            ticks: Expr::Num(0),
            meters: Vec::new(),
            items: Vec::new(),
        },
    };
    let mut diagnostics = Diagnostics::default();
    let if_ = (entry.if_.iter())
        .filter_map(|cond| {
            let push_error = |error| {
                diagnostics.push(file_text.refuse_error(&format!("paths[{index}].if"), error))
            };
            rule_list::compile::cond(cond).map_err(push_error).ok()
        })
        .collect();
    diagnostics.into_result(())?;
    let path_def = PathDef {
        id,
        name: entry.name.clone(),
        end_place_ids,
        bidirectional,
        cost,
        if_,
        locked_text: entry.locked_text.clone(),
        revealed_by: entry.revealed_by.clone(),
    };
    if let Some((key, _)) = path_def.mentions().find_misplaced_target(TargetKind::None) {
        let message = format!(
            "path '{}' says target in {key}, but an exit has no target",
            entry.path
        );
        return Err(refuse(
            "",
            &message,
            "an exit is walked, not targeted; name self or an id",
        ));
    }
    Ok(path_def)
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

/// The one flag a path with an `if` carries, set once someone at either end meets the `if`.
pub(crate) const OPEN: &str = "open";

/// What the pack says about one path.
#[derive(Clone, Debug)]
pub struct PathDef {
    /// The pack's `id`, or `<first place id>.<second place id>` when it leaves it out.
    pub id: PathId,
    /// What a player reads for it at either end. Left out, each end reads the place it goes to.
    pub name: Option<String>,
    /// The two places the entry names, in its order.
    pub end_place_ids: [PlaceId; 2],
    /// The entry says `<->`.
    pub bidirectional: bool,
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

    /// Every number the path holds: its cost's, then its conds'.
    pub(super) fn exprs(&self) -> Vec<&Expr> {
        let cond_numbers = self.if_.iter().flat_map(Cond::exprs);
        self.cost.exprs().chain(cond_numbers).collect()
    }

    /// The entities, item ids and `target`s the path's conds, cost and numbers name.
    pub(super) fn mentions(&self) -> mentions::Mentions<'_> {
        mentions::Mentions::of_lines(&self.if_, [&self.cost], self.exprs())
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

/// A path as seen from a place it leaves.
#[derive(Clone, Copy, Debug)]
pub struct Exit<'c> {
    pub path_def: &'c PathDef,
    pub from_place_id: &'c PlaceId,
}

impl<'c> Exit<'c> {
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
    pub fn walker_location(self) -> ActorLocation {
        ActorLocation::Path {
            path_id: self.path_def.id.clone(),
            to_place_id: self.to_place_id().clone(),
        }
    }

    /// The name a player reads: its path's name, or else the name of the place it goes to. A
    /// loaded canon has a place at every end, so the empty name never shows.
    pub(crate) fn name(self, canon: &'c Canon) -> &'c str {
        match &self.path_def.name {
            Some(name) => name,
            None => (canon.places.get(self.to_place_id())).map_or("", |place| &place.name),
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
    /// Index each path's id and each place's exits from `path_defs`, in file order: a path goes
    /// under its first place, and under its second too when it goes both ways. Two paths with one
    /// id are refused.
    pub(super) fn new(path_defs: Vec<PathDef>) -> Result<Self, Vec<Diagnostic>> {
        let mut exit_def_indexes_by_place_id: BTreeMap<PlaceId, Vec<usize>> = BTreeMap::new();
        let mut def_index_by_path_id = BTreeMap::new();
        let mut diagnostics = Diagnostics::default();
        for (index, path_def) in path_defs.iter().enumerate() {
            if def_index_by_path_id
                .insert(path_def.id.clone(), index)
                .is_some()
            {
                let id_from_ends = path_def.id.as_str().contains('.');
                let field = if id_from_ends { ".path" } else { ".id" };
                let message = format!("path id '{}' is used twice", path_def.id);
                let key_path = format!("paths[{index}]{field}");
                let hint = "give one of these paths its own id";
                diagnostics.push(schema_error(FILE, &message, hint).at_key_path(key_path));
            }
            let ends = path_def.end_place_ids.iter();
            for from_place_id in ends.take(if path_def.bidirectional { 2 } else { 1 }) {
                (exit_def_indexes_by_place_id.entry(from_place_id.clone()))
                    .or_default()
                    .push(index);
            }
        }
        diagnostics.into_result(())?;
        Ok(Self {
            path_defs,
            exit_def_indexes_by_place_id,
            def_index_by_path_id,
        })
    }

    pub fn get(&self, id: &PathId) -> Option<&PathDef> {
        (self.def_index_by_path_id.get(id)).map(|&index| &self.path_defs[index])
    }

    /// Every path's id, in id order.
    pub(crate) fn ids(&self) -> impl Iterator<Item = &PathId> {
        self.def_index_by_path_id.keys()
    }

    /// The exits of `from_place_id`, in the file order of their paths. The place id the iterator
    /// reads is the index's own key, so it borrows only `self`.
    pub(crate) fn exits(&self, from_place_id: &str) -> impl Iterator<Item = Exit<'_>> {
        let entry = self
            .exit_def_indexes_by_place_id
            .get_key_value(from_place_id);
        entry.into_iter().flat_map(|(place_id, def_indexes)| {
            (def_indexes.iter())
                .filter_map(|&index| self.path_defs[index].exit_from(place_id.as_str()))
        })
    }

    /// Each path, in file order.
    pub fn iter(&self) -> impl Iterator<Item = &PathDef> {
        self.path_defs.iter()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Yaml {
    #[serde(rename = "schema")]
    _schema: String,
    paths: Vec<PathYaml>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PathYaml {
    path: String,
    id: Option<String>,
    name: Option<String>,
    cost: Option<rule_list::parse::CostYaml>,
    #[serde(default, rename = "if")]
    if_: Vec<rule_list::parse::CondYaml>,
    #[serde(default)]
    locked_text: String,
    revealed_by: Option<String>,
}
