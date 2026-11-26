//! The loaded pack: places, cast, and rules.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::diagnostic::Diagnostic;
use crate::ecs::component::meter;
use crate::ecs::component::modifier;
use crate::ecs::id::{ActorId, PlaceId, StoryId};
use crate::entity;
use crate::hash::blake2s_tagged;
use crate::rules::rule::{self, RuleKind, Scope, TargetKind};
use crate::Reason;
use spookery_protocol::{MenuRow, PackSummary};

pub(crate) mod budget;
mod cast;
pub(crate) mod item;
pub(crate) mod lint;
pub(crate) mod map;
mod meters;
mod modifiers;
mod pack;
mod pack_files;
mod place;
mod refusal;
mod rule_list;
pub(crate) mod story;
mod verbs;

use budget::Budget;
use lint::entity_defs::{find_missing_components, find_unknown_meter_reads, EntityDefs};
use lint::{find_unknown_modifiers, find_unknown_path_ids, find_unknown_rule_items};
use map::Paths;
use pack_files::{collect_files, collect_markdown_files, read_text};
use refusal::{Diagnostics, FileText, Locator};

#[derive(Clone, Debug)]
pub struct Canon {
    pub pack_id: String,
    pub title: String,
    pub system: String,
    start_place: String,
    pub places: BTreeMap<PlaceId, place::Def>,
    /// Every entry under `paths:` in `world/map.yaml`, in file order, and the index of each
    /// place's exits.
    pub paths: Paths,
    pub cast: BTreeMap<ActorId, cast::Def>,
    pub(crate) stories: BTreeMap<StoryId, story::Def>,
    pub rules: rule::Table,
    /// The meters `rules/meters.yaml` gives every actor, and those one cast, place or story
    /// file gives its entity. An id is listed once.
    pub meters: BTreeMap<String, meter::Def>,
    /// The meters `world/meters.yaml` gives the world. An id here may also be an actor meter.
    pub(crate) world_meters: BTreeMap<String, meter::Def>,
    /// The modifiers `rules/modifiers.yaml` defines. Each one modifies an actor meter.
    pub modifiers: BTreeMap<String, modifier::Def>,
    pub items: BTreeMap<String, item::Def>,
    pub(crate) budget: Budget,
    /// Rounds between journal checkpoints. `save.yaml` is rewritten then.
    pub(crate) checkpoint_every: u64,
    /// `gm_notes` in `pack.yaml`: tone and flavor for the GM, empty when the pack has none.
    gm_notes: String,
}

impl Canon {
    /// Every verb some rule names, sorted, once each.
    pub(crate) fn verbs(&self) -> Vec<String> {
        let verbs: std::collections::BTreeSet<&str> = self
            .rules
            .iter()
            .filter_map(|rule| match &rule.kind {
                RuleKind::Verb { verb_id, .. } => Some(verb_id.as_str()),
                _ => None,
            })
            .collect();
        verbs.into_iter().map(str::to_string).collect()
    }

    pub(crate) fn to_pack_summary(&self) -> PackSummary {
        let menu = self
            .rules
            .iter()
            .filter_map(|rule| match &rule.kind {
                RuleKind::Verb { verb_id, target } if matches!(rule.scope, Scope::Verb) => {
                    Some((verb_id.clone(), "verb", target.as_str()))
                }
                RuleKind::Habit => {
                    Some((rule.local_id.clone(), "habit", TargetKind::None.as_str()))
                }
                _ => None,
            })
            .map(|(id, audience, token)| MenuRow {
                id,
                kind: audience.to_string(),
                target: token.to_string(),
            })
            .collect();
        let control_types = self
            .cast
            .iter()
            .map(|(k, m)| (k.to_string(), m.control_type))
            .collect();
        let clip = |s: &str| s.chars().take(800).collect();
        let places = self
            .places
            .iter()
            .map(|(k, p)| (k.to_string(), clip(&p.body)))
            .collect();
        let cast = self
            .cast
            .iter()
            .map(|(k, m)| (k.to_string(), clip(&m.body)))
            .collect();
        PackSummary {
            menu,
            control_types,
            places,
            cast,
            gm_notes: self.gm_notes.clone(),
        }
    }

    /// The `meter::Def` of meter `key` on `id`, when that entity carries it.
    pub(crate) fn meter_def(&self, id: entity::IdRef<'_>, key: &str) -> Option<&meter::Def> {
        match id {
            entity::IdRef::Global => self.world_meters.get(key),
            _ => self.meters.get(key).filter(|def| def.is_on(id)),
        }
    }

    pub(crate) fn idle_profile(&self, actor_id: &ActorId) -> Option<&cast::IdleProfile> {
        self.cast.get(actor_id).and_then(|cast| cast.idle.as_ref())
    }

    /// The name a player reads for the cast member, place, item or story `id`, or `None` when
    /// the pack has no such id.
    pub fn name(&self, id: &str) -> Option<&str> {
        let cast = self.cast.get(id).map(|cast| &cast.name);
        let place = || self.places.get(id).map(|place| &place.name);
        let item = || self.items.get(id).map(|item| &item.name);
        let story = || self.stories.get(id).map(|story| &story.name);
        cast.or_else(place)
            .or_else(item)
            .or_else(story)
            .map(String::as_str)
    }
}

pub fn pack_rev(root: &Path) -> Result<String, Reason> {
    let mut files = Vec::new();
    for dir in ["world", "cast", "rules"] {
        collect_files(&root.join(dir), root, &mut files)?;
    }
    files.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    let mut preimage = Vec::new();
    for (file, bytes) in files {
        preimage.extend(file.as_bytes());
        preimage.push(0);
        preimage.extend((bytes.len() as u64).to_be_bytes());
        preimage.extend(bytes);
    }
    Ok(blake2s_tagged(&preimage))
}

/// Loads a pack and reports its diagnostics in load order: `pack.yaml`; the items and places
/// under `world/`, `world/map.yaml` and the optional `world/meters.yaml`; `rules/meters.yaml`, then each cast and
/// place file's own meters; the optional `rules/modifiers.yaml`; a missing `rules/verbs.yaml` or
/// `rules/budget.yaml`; verbs; `story/` and each story's own meters; rule ids across files;
/// the budget; lints across files; and `checkpoint_every`.
pub fn load_canon(root: &Path) -> Result<Canon, Vec<Diagnostic>> {
    // repeats module: re-exported at the crate root
    let mut diagnostics = Diagnostics::default();
    let mut places = BTreeMap::new();
    let mut cast = BTreeMap::new();
    let mut items = BTreeMap::new();
    let mut place_meters = Vec::new();
    let mut unsorted_rules = Vec::new();
    let pack = diagnostics.keep(read_text(root, "pack.yaml").and_then(|text| pack::load(&text)));
    for page in collect_markdown_files(root, "world") {
        let Some(page) = diagnostics.keep(page) else {
            continue;
        };
        let rules = if page.file.starts_with("world/items/") {
            item::add(page, &mut items, &mut diagnostics)
        } else if page.file.starts_with("world/places/") {
            place::add(page, &mut places, &mut place_meters, &mut diagnostics)
        } else {
            None
        };
        unsorted_rules.extend(rules.unwrap_or_default());
    }
    let path_defs = diagnostics.keep(read_text(root, map::FILE).and_then(|text| map::load(&text)));
    let paths =
        (path_defs.as_ref()).and_then(|path_defs| diagnostics.keep(Paths::new(path_defs.clone())));
    let world_meters = if root.join("world/meters.yaml").is_file() {
        let file = "world/meters.yaml";
        diagnostics.keep(
            read_text(root, file)
                .and_then(|text| meters::load(file, &text, Some(entity::Id::Global))),
        )
    } else {
        Some(BTreeMap::new())
    };
    let file = "rules/meters.yaml";
    let mut meters =
        diagnostics.keep(read_text(root, file).and_then(|text| meters::load(file, &text, None)));
    for page in collect_markdown_files(root, "cast") {
        let Some(page) = diagnostics.keep(page) else {
            continue;
        };
        let rules = cast::add(page, &mut cast, meters.as_mut(), &mut diagnostics);
        unsorted_rules.extend(rules.unwrap_or_default());
    }
    for (file, own) in place_meters {
        if let Some(meters) = meters.as_mut() {
            meters::add_own(
                meters,
                own,
                None,
                &FileText::without_text(&file),
                &mut diagnostics,
            );
        }
    }
    let modifiers = match &meters {
        Some(meters) if root.join(modifiers::FILE).is_file() => diagnostics
            .keep(read_text(root, modifiers::FILE).and_then(|text| modifiers::load(&text, meters))),
        _ => Some(BTreeMap::new()),
    };
    let [verbs, budget] = ["rules/verbs.yaml", "rules/budget.yaml"].map(|file| {
        diagnostics
            .keep(read_text(root, file))
            .map(|text| (file, text))
    });
    if let Some((file, text)) = &verbs {
        unsorted_rules.extend(
            diagnostics
                .keep(verbs::load(file, text))
                .unwrap_or_default(),
        );
    }
    let (story_rules, stories) = story::load_dir(
        root,
        meters.as_mut(),
        world_meters.as_ref(),
        &mut diagnostics,
    );
    unsorted_rules.extend(story_rules);
    let mut seen_ids = BTreeSet::new();
    for rule in &unsorted_rules {
        let duplicate_words = ("rule", "is already defined", "keep one rule with this id");
        let is_duplicate = !seen_ids.insert(rule.id.clone());
        let file_text = FileText::without_text(&rule.file);
        diagnostics.extend(
            is_duplicate.then(|| {
                file_text.refuse_duplicate(&rule.id, "", Locator::Nowhere, duplicate_words)
            }),
        );
    }
    let rules = rule::Table::new(unsorted_rules);
    let budget = budget.and_then(|(file, text)| diagnostics.keep(budget::load(file, &text)));
    if let (Some(meters), Some(world_meters), Some(paths)) = (&meters, &world_meters, &paths) {
        let entity_defs = EntityDefs {
            meters,
            world_meters,
            cast: &cast,
            places: &places,
            paths,
            stories: &stories,
        };
        diagnostics.extend(find_unknown_meter_reads(&entity_defs, &rules));
        diagnostics.extend(find_missing_components(&entity_defs, &rules));
    }
    if let Some(modifiers) = &modifiers {
        diagnostics.extend(find_unknown_modifiers(&rules, modifiers));
    }
    diagnostics.extend(find_unknown_rule_items(&rules, &items));
    if let Some(path_defs) = &path_defs {
        diagnostics.extend(find_unknown_path_ids(path_defs, &places, &items));
    }
    let checkpoint_every = pack
        .as_ref()
        .and_then(|pack| diagnostics.keep(pack.checkpoint_every()));
    diagnostics.into_result(())?;
    let (
        Some(pack),
        Some(paths),
        Some(meters),
        Some(world_meters),
        Some(modifiers),
        Some(budget),
        Some(checkpoint_every),
    ) = (
        pack,
        paths,
        meters,
        world_meters,
        modifiers,
        budget,
        checkpoint_every,
    )
    else {
        unreachable!("a part that did not load reported why");
    };
    Ok(Canon {
        pack_id: pack.id,
        title: pack.title,
        system: pack.system,
        start_place: pack.start.place,
        places,
        paths,
        cast,
        stories,
        rules,
        meters,
        world_meters,
        modifiers,
        items,
        budget,
        checkpoint_every,
        gm_notes: pack.gm_notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_rows_keep_verbs_and_habits_and_control_types() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/packs/docks");
        let mut canon = load_canon(&root).expect("docks canon");
        canon.cast.get_mut("mira").unwrap().body = "CAST-BODY-SENTINEL".to_string();
        let summary = canon.to_pack_summary();
        assert!(summary.menu.iter().all(|row| row.kind != "system"));
        assert!(summary
            .menu
            .iter()
            .any(|row| row.id == "travel" && row.kind == "verb" && row.target == "path"));
        assert!(summary
            .menu
            .iter()
            .any(|row| row.id == "restock_lamp_oil" && row.kind == "habit"));
        assert_eq!(
            summary.control_types.get("mira"),
            Some(&spookery_protocol::ControlType::Npc)
        );
        assert_eq!(
            summary.control_types.get("rowan"),
            Some(&spookery_protocol::ControlType::PcTemplate)
        );
        let rendered = format!("{:?}{:?}", summary.menu, summary.control_types);
        assert!(!rendered.contains("CAST-BODY-SENTINEL"));
        assert_eq!(
            summary.cast.get("mira").map(String::as_str),
            Some("CAST-BODY-SENTINEL")
        );
    }
}
