//! The loaded pack: places, cast, facts, and rules.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::Deserialize;

use crate::canon;
use crate::diagnostic::Diagnostic;
use crate::ecs::component::meter;
use crate::ecs::component::modifier;
use crate::ecs::id::{ActorId, ExitId, PlaceId, StoryId};
use crate::entity;
use crate::hash::blake2s_tagged;
use crate::rules::expr::Expr;
use crate::rules::rule::{self, Cond, Cost, Owner, TargetKind, When};
use crate::Reason;
pub use spookery_protocol::{ControlType, MenuRow, PackSummary};

mod budget;
mod cast;
mod fact;
mod item;
mod lint;
mod meters;
mod modifiers;
mod pack;
mod place;
mod rule_list;
mod source;
mod story;

use budget::load_budget;
pub(crate) use budget::Budget;
use fact::Facts;
use item::add_item;
use lint::{
    broken_exits, reveal_problems, unknown_carriers, unknown_modifiers, unknown_reads,
    unknown_rule_items, EntityScope,
};
pub(crate) use lint::{check_canon, unknown_item};
use modifiers::MODIFIERS_FILE;
use pack::load_pack_yaml;
use place::name_exits;
use rule_list::load_verbs;
use source::{collect_files, collect_markdown_files, read_text, Diags, FileText, Locator};
use story::load_stories;

#[derive(Clone, Debug)]
pub struct Canon {
    pub pack_id: String,
    pub title: String,
    pub system: String,
    pub start_place: String,
    pub places: BTreeMap<PlaceId, PlaceDef>,
    pub cast: BTreeMap<ActorId, CastDef>,
    pub facts: BTreeMap<String, FactDef>,
    pub stories: BTreeMap<StoryId, StoryDef>,
    pub rules: rule::Table,
    /// The meters `rules/meters.yaml` gives every actor, and those one cast, place or story
    /// file gives its entity. An id is listed once.
    pub meters: BTreeMap<String, meter::Descriptor>,
    /// The meters `world/meters.yaml` gives the world. An id here may also be an actor meter.
    pub world_meters: BTreeMap<String, meter::Descriptor>,
    /// The modifiers `rules/modifiers.yaml` defines. Each one modifies an actor meter.
    pub modifiers: BTreeMap<String, modifier::Descriptor>,
    pub items: BTreeMap<String, ItemDef>,
    pub budget: Budget,
    /// Rounds between journal checkpoints. `save.yaml` is rewritten then.
    pub checkpoint_every: u64,
    /// `gm_notes` in `pack.yaml`: tone and flavor for the GM, empty when the pack has none.
    pub gm_notes: String,
}

#[derive(Clone, Debug)]
pub struct ItemDef {
    /// The name a player reads, the id when the pack gives none.
    pub name: String,
    pub tags: Vec<String>,
    pub body: String,
}

#[derive(Clone, Debug)]
pub struct FactDef {
    pub text: String,
    pub hidden: bool,
    pub place_id: String,
}

/// The one flag an exit with an `if` carries, set once someone there meets the `if`.
pub(crate) const OPEN: &str = "open";

/// One way out of a place. `cost.ticks` is the game time the walk takes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exit {
    /// Unique among this place's exits.
    pub id: String,
    /// The name a player reads and types. Left out by the pack, it is the name of the place it
    /// goes to when `id` is that place's id, or else `id`.
    pub name: String,
    /// The place this exit leaves.
    pub from: PlaceId,
    pub to: String,
    pub cost: Cost,
    pub if_: Vec<Cond>,
    pub locked_text: String,
    pub hidden: bool,
    pub revealed_by: Option<String>,
    pub file: String,
    pub index: usize,
}

impl Exit {
    /// This exit's door, by its place and its id.
    pub(crate) fn qualified_id(&self) -> ExitId {
        ExitId {
            place: self.from.clone(),
            exit: self.id.clone(),
        }
    }

    /// Every number the exit holds: its cost's, then its conditions' values.
    pub(crate) fn exprs(&self) -> Vec<&Expr> {
        let values = self.if_.iter().flat_map(Cond::exprs);
        self.cost.exprs().chain(values).collect()
    }

    /// Every item id the exit's conditions and cost name, with its slot.
    pub(crate) fn item_ids(&self) -> Vec<(&'static str, &str)> {
        rule::collect_item_ids(&self.if_, [&self.cost])
    }
}

#[derive(Clone, Debug)]
pub struct PlaceDef {
    /// The name a player reads, the id when the pack gives none.
    pub name: String,
    pub body: String,
    pub exits: Vec<Exit>,
    /// The flags an effect may set on this place.
    pub flags: Vec<String>,
}

impl PlaceDef {
    /// Whether exit `exit_id` here is a door: it has an `if`, so it carries the flag `open`.
    pub(crate) fn has_door(&self, exit_id: &str) -> bool {
        self.exits
            .iter()
            .any(|exit| exit.id == exit_id && !exit.if_.is_empty())
    }
}

#[derive(Clone, Debug)]
pub struct StoryDef {
    /// The name a player reads, the id when the pack gives none.
    pub name: String,
    /// The flags an effect may set on this story.
    pub flags: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct CastDef {
    pub control_type: canon::ControlType,
    /// The name a player reads, the id when the pack gives none.
    pub name: String,
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
    /// Every verb some rule names, sorted, once each.
    pub fn verbs(&self) -> Vec<String> {
        let verbs: std::collections::BTreeSet<&str> = self
            .rules
            .iter()
            .filter_map(|rule| match &rule.when {
                When::Verb { verb_id, .. } => Some(verb_id.as_str()),
                _ => None,
            })
            .collect();
        verbs.into_iter().map(str::to_string).collect()
    }

    pub fn to_pack_summary(&self) -> PackSummary {
        let menu = self
            .rules
            .iter()
            .filter_map(|rule| match &rule.when {
                When::Verb { verb_id, target } if matches!(rule.owner, Owner::Verb) => {
                    Some((verb_id.clone(), "verb", target.as_str()))
                }
                When::Habit => Some((rule.local_id.clone(), "habit", TargetKind::None.as_str())),
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

    /// The descriptor of meter `key` on `id`, when that entity carries it.
    pub fn meter_desc(&self, id: entity::IdRef<'_>, key: &str) -> Option<&meter::Descriptor> {
        match id {
            entity::IdRef::Global => self.world_meters.get(key),
            _ => self.meters.get(key).filter(|desc| desc.is_on(id)),
        }
    }

    /// Whether `id` carries flag `key`: one its file declares, or any world flag but `busy`.
    pub(crate) fn carries_flag(&self, id: entity::IdRef<'_>, key: &str) -> bool {
        match declared_flags(&self.cast, &self.places, &self.stories, id) {
            Some(flags) => flags.iter().any(|flag| flag == key),
            None => !key.is_empty() && key != "busy",
        }
    }

    /// Whether `exit` is a door: an exit of its place with an `if`, so it can be open or shut.
    pub(crate) fn has_door(&self, exit: &ExitId) -> bool {
        (self.places.get(&exit.place)).is_some_and(|at| at.has_door(&exit.exit))
    }

    pub fn idle_profile(&self, actor_id: &ActorId) -> Option<&IdleProfile> {
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

    pub(crate) fn body_of_fact(&self, fact_id: &str) -> &str {
        let Some(fact) = self.facts.get(fact_id) else {
            return "";
        };
        self.places
            .get(fact.place_id.as_str())
            .map_or("", |place| &place.body)
    }
}

/// The flags `carrier`'s file declares, none for an id the pack lacks: `None` for the world,
/// which carries any flag but `busy`. Lint asks before the canon is assembled, so it takes the
/// three maps.
pub(crate) fn declared_flags<'a>(
    cast: &'a BTreeMap<ActorId, CastDef>,
    places: &'a BTreeMap<PlaceId, PlaceDef>,
    stories: &'a BTreeMap<StoryId, StoryDef>,
    carrier: entity::IdRef<'_>,
) -> Option<&'a [String]> {
    let flags = match carrier {
        entity::IdRef::Actor(id) => cast.get(id).map(|cast| &cast.flags),
        entity::IdRef::Place(id) => places.get(id).map(|place| &place.flags),
        entity::IdRef::Story(id) => stories.get(id).map(|story| &story.flags),
        entity::IdRef::Global => return None,
    };
    Some(flags.map_or(&[], Vec::as_slice))
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
    Ok(blake2s_tagged(&preimage))
}

/// Loads a pack in the numbered order below, which is also the order of its diagnostics.
pub fn load_canon(root: &Path) -> Result<Canon, Vec<Diagnostic>> {
    // repeats module: re-exported at the crate root
    let mut diags = Diags::default();
    let mut places = BTreeMap::new();
    let mut cast = BTreeMap::new();
    let mut facts = Facts::default();
    let mut items = BTreeMap::new();
    let mut place_meters = Vec::new();
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
            place::add(page, &mut facts, &mut places, &mut place_meters, &mut diags)
        } else {
            None
        };
        loaded.extend(rules.unwrap_or_default());
    }
    name_exits(&mut places);
    // world/meters.yaml is optional.
    let world_meters = if root.join("world/meters.yaml").is_file() {
        let rel = "world/meters.yaml";
        diags.keep(
            read_text(root, rel)
                .and_then(|text| meters::load(rel, &text, Some(entity::Id::Global))),
        )
    } else {
        Some(BTreeMap::new())
    };
    // 3. rules/meters.yaml, then one walk of cast/, which adds each cast file's own meters,
    // then each place file's own meters.
    let rel = "rules/meters.yaml";
    let mut meters =
        diags.keep(read_text(root, rel).and_then(|text| meters::load(rel, &text, None)));
    for page in collect_markdown_files(root, "cast") {
        let Some(page) = diags.keep(page) else {
            continue;
        };
        let rules = cast::add(page, &mut cast, meters.as_mut(), &mut diags);
        loaded.extend(rules.unwrap_or_default());
    }
    for (file, own) in place_meters {
        if let Some(meters) = meters.as_mut() {
            meters::add_own(meters, own, None, &FileText::named(&file), &mut diags);
        }
    }
    // rules/modifiers.yaml is optional. Its modifiers name the actor meters above.
    let modifiers = match &meters {
        Some(meters) if root.join(MODIFIERS_FILE).is_file() => diags
            .keep(read_text(root, MODIFIERS_FILE).and_then(|text| modifiers::load(&text, meters))),
        _ => Some(BTreeMap::new()),
    };
    // 4. rules/moves.yaml is refused.
    if root.join("rules/moves.yaml").is_file() {
        diags.push(rule_list::replaced_moves_file());
    }
    // 5. The other two rule files are read in this order; a missing one is reported here.
    let [verbs, budget] = ["rules/verbs.yaml", "rules/budget.yaml"]
        .map(|rel| diags.keep(read_text(root, rel)).map(|text| (rel, text)));
    // 6. Verbs.
    if let Some((rel, text)) = &verbs {
        loaded.extend(diags.keep(load_verbs(rel, text)).unwrap_or_default());
    }
    // 7. story/, which adds each story file's own meters.
    let (story_rules, stories) =
        load_stories(root, meters.as_mut(), world_meters.as_ref(), &mut diags);
    loaded.extend(story_rules);
    // 8. Rule ids across files.
    let mut seen_ids = BTreeSet::new();
    for rule in &loaded {
        let twice = ("rule", "is already defined", "keep one rule with this id");
        let taken = !seen_ids.insert(rule.id.clone());
        let source = FileText::named(&rule.file);
        diags.extend(taken.then(|| source.refuse_duplicate(&rule.id, "", Locator::Nowhere, twice)));
    }
    let rules = rule::Table::new(loaded);
    // 9. Budget.
    let budget = budget.and_then(|(rel, text)| diags.keep(load_budget(rel, &text)));
    // 10. Lints across files.
    if let (Some(meters), Some(world_meters)) = (&meters, &world_meters) {
        let scope = EntityScope {
            meters,
            world_meters,
            cast: &cast,
            places: &places,
            stories: &stories,
        };
        diags.extend(unknown_reads(&scope, &rules));
        diags.extend(unknown_carriers(&scope, &rules));
    }
    if let Some(modifiers) = &modifiers {
        diags.extend(unknown_modifiers(&rules, modifiers));
    }
    diags.extend(unknown_rule_items(&rules, &items));
    diags.extend(reveal_problems(&rules, &facts.0));
    diags.extend(broken_exits(&places, &facts.0, &items));
    // 11. checkpoint_every.
    let checkpoint_every = pack
        .as_ref()
        .and_then(|pack| diags.keep(pack.checkpoint_every()));
    diags.into_result(())?;
    let (
        Some(pack),
        Some(meters),
        Some(world_meters),
        Some(modifiers),
        Some(budget),
        Some(checkpoint_every),
    ) = (
        pack,
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
        cast,
        facts: facts.0,
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

/// Markdown with the body removed. Frontmatter stays. The trailing delimiter is kept.
#[cfg(any(test, feature = "testing"))]
pub fn strip_body(text: &str) -> Result<String, Reason> {
    let (yaml, _) = source::split_frontmatter("", text).map_err(|_| Reason::Schema)?;
    Ok(format!("---\n{yaml}\n---\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Reason;

    #[test]
    fn strip_body_rejects_a_missing_fence() {
        assert_eq!(strip_body("no fence\n"), Err(Reason::Schema));
        assert_eq!(strip_body("---\nno closer\n"), Err(Reason::Schema));
    }

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
            .any(|row| row.id == "travel" && row.kind == "verb" && row.target == "exit"));
        assert!(summary
            .menu
            .iter()
            .any(|row| row.id == "restock_lamp_oil" && row.kind == "habit"));
        assert_eq!(
            summary.control_types.get("mira"),
            Some(&canon::ControlType::Npc)
        );
        assert_eq!(
            summary.control_types.get("rowan"),
            Some(&canon::ControlType::PcTemplate)
        );
        let rendered = format!("{:?}{:?}", summary.menu, summary.control_types);
        assert!(!rendered.contains("CAST-BODY-SENTINEL"));
        assert_eq!(
            summary.cast.get("mira").map(String::as_str),
            Some("CAST-BODY-SENTINEL")
        );
    }
}
