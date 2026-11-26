//! Read, write, migrate and check the save.

use std::collections::BTreeSet;

use serde::Deserialize;
use serde_saphyr::SerializerOptions;

use crate::canon::item;
use crate::canon::lint::refuse_unknown_item;
use crate::diagnostic::{
    diagnose_schema_tag, error, locate_nth, parse_file, schema_error, Diagnostic,
};
use crate::ecs::component::meter::{self, Meter};
use crate::ecs::component::modifier::{self, Modifier};
use crate::ecs::id::ActorId;
use crate::ecs::{self, component, Save};
use crate::entity;
use crate::store::records::{DroppedMeter, MigrateRecord};
use crate::Reason;

const FILE: &str = "state/save.yaml";

pub(crate) fn emit_options() -> SerializerOptions {
    serde_saphyr::ser_options! {
        indent_step: 2,
        compact_list_indent: false,
        anchor_generator: None,
        min_fold_chars: usize::MAX,
        folded_wrap_chars: usize::MAX,
        tagged_enums: false,
        prefer_block_scalars: false,
        quote_all: false,
        yaml_12: false,
    }
}

/// Reads a save. A save from older rules, even one with a field these rules dropped, such as
/// `presence`, is refused as `RULES_REV_MISMATCH`.
pub fn parse(text: &str) -> Result<Save, Vec<Diagnostic>> {
    if let Some(diagnostic) = diagnose_schema_tag(text, crate::diagnostic::SAVE_SCHEMA, FILE) {
        return Err(vec![diagnostic]);
    }
    #[derive(Deserialize)]
    struct RulesProbe {
        #[serde(default)]
        rules_rev: u64,
    }
    if let Ok(probe) = serde_saphyr::from_str::<RulesProbe>(text) {
        if super::require_rules_rev(probe.rules_rev).is_err() {
            return Err(vec![rules_rev_diagnostic(text)]);
        }
    }
    parse_file(FILE, text, false)
}

pub fn emit(save: &Save) -> Result<String, Reason> {
    let mut canonical = save.clone();
    canonical.schema = crate::diagnostic::SAVE_SCHEMA.to_string();
    canonical.rules_rev = super::RULES_REV;
    let text = serde_saphyr::to_string_with_options(&canonical, emit_options())
        .map_err(|_| Reason::Schema)?;
    Ok(one_trailing_lf(text))
}

fn rules_rev_diagnostic(text: &str) -> Diagnostic {
    let mut diagnostic = error(
        FILE,
        Reason::RulesRevMismatch,
        "this save was written by different rules",
        "start a fresh save",
    )
    .at_key_path("rules_rev");
    diagnostic = diagnostic.at_file_location(locate_nth(text, "rules_rev:", 0));
    diagnostic
}

fn one_trailing_lf(mut text: String) -> String {
    while text.ends_with('\n') {
        text.pop();
    }
    text.push('\n');
    text
}

/// Every meter the save lists, with the actor, place, story or world that holds it.
fn saved_meters(world: &ecs::World) -> impl Iterator<Item = (entity::Id, &str, &Meter)> {
    let actors = world.actors().query::<&Meter>();
    let places = world.places().query::<&Meter>();
    let stories = world.stories().query::<&Meter>();
    let globals = world.globals().query::<&Meter>();
    let actors = (actors.iter().flat_map(|entry| entry.iter_with_id()))
        .map(|(id, key, meter)| (entity::Id::Actor(id.clone()), key, meter));
    let places = (places.iter().flat_map(|entry| entry.iter_with_id()))
        .map(|(id, key, meter)| (entity::Id::Place(id.clone()), key, meter));
    let stories = (stories.iter().flat_map(|entry| entry.iter_with_id()))
        .map(|(id, key, meter)| (entity::Id::Story(id.clone()), key, meter));
    let globals = (globals.iter().flat_map(|entry| entry.iter_with_id()))
        .map(|(_, key, meter)| (entity::Id::Global, key, meter));
    actors.chain(places).chain(stories).chain(globals)
}

/// Every flag key the save lists, on any actor, place, path, story or the world.
pub(crate) fn find_saved_flags(world: &ecs::World) -> BTreeSet<&str> {
    let actors = world.actors().query::<&component::Flag>();
    let places = world.places().query::<&component::Flag>();
    let paths = world.paths().query::<&component::Flag>();
    let stories = world.stories().query::<&component::Flag>();
    let globals = world.globals().query::<&component::Flag>();
    let keys = (actors.iter().flat_map(|flags| flags.iter()))
        .chain(places.iter().flat_map(|flags| flags.iter()))
        .chain(paths.iter().flat_map(|flags| flags.iter()))
        .chain(stories.iter().flat_map(|flags| flags.iter()))
        .chain(globals.iter().flat_map(|flags| flags.iter()));
    keys.map(|(key, ())| key).collect()
}

/// Meters an actor, place, story or the world lists that the pack no longer gives it.
pub(crate) fn find_stale_meters(world: &ecs::World) -> Vec<DroppedMeter> {
    saved_meters(world)
        .filter(|(entity_id, id, _)| world.canon().meter_def(entity_id.as_ref(), id).is_none())
        .map(|(entity_id, id, _)| DroppedMeter {
            id: id.to_string(),
            entity_id,
        })
        .collect()
}

/// Each actor's modifiers that the pack no longer defines, or now defines on another meter:
/// the actor, the modifier id and the modifier as the save holds it.
pub(crate) fn find_stale_modifiers(world: &ecs::World) -> Vec<(&ActorId, &str, &Modifier)> {
    let actors = world.actors().query::<&Modifier>();
    let modifiers = actors.iter().flat_map(|entry| entry.iter_with_id());
    modifiers
        .filter(|(_, id, modifier)| !modifier::is_defined(&world.canon().modifiers, id, modifier))
        .collect()
}

/// Migrate a save as `record` says, through the writers a round uses: take the dropped modifiers
/// off every actor and each dropped meter off its entity, then stamp the record's `pack_rev_after`
/// and `state_rev_after`. A meter an entity holds keeps its value, and one it has no row for reads
/// the pack's base. Live migrate and replay both call this. It does not touch the disk, and it
/// reads no hash or chain from the record.
pub(crate) fn migrate(mut world: ecs::World, record: &MigrateRecord) -> Result<ecs::World, Reason> {
    let actors = world.actors().query::<&Modifier>();
    let modifiers = actors.iter().flat_map(|entry| entry.iter_with_id());
    let dropped =
        modifiers.filter(|(_, id, _)| record.dropped_modifiers.iter().any(|drop| drop == id));
    let dropped: Vec<(ActorId, String)> =
        (dropped.map(|(actor, id, _)| (actor.clone(), id.to_string()))).collect();
    for (actor, id) in &dropped {
        (world.actors_mut().get_mut::<Modifier>(actor)).set(id, None)?;
    }
    for meter in &record.dropped_meters {
        (world.entities_mut().get_mut::<Meter>(&meter.entity_id)).set(&meter.id, None)?;
    }
    world.pack_rev = record.pack_rev_after.clone();
    world.state_rev = record.state_rev_after;
    world.accept();
    Ok(world)
}

/// Check the save against the pack without changing it. Refuse every component on an actor with no
/// control type, every actor the cast lacks, every actor without a place, every actor in a place
/// or on a path the pack lacks, every meter, item and wielded slot the pack does not define,
/// every meter outside its rules, every modifier the pack no longer gives that meter, and every
/// empty id or zero count.
pub(crate) fn check(world: &ecs::World) -> Result<(), Vec<Diagnostic>> {
    let canon = world.canon();
    let mut diagnostics = Vec::new();
    let actors = world.actors();
    let ids_by_store = actors.ids_by_store();
    let lacking = ids_by_store.iter().filter(|(_, id)| !actors.contains(id));
    for (store, id) in lacking {
        let message = format!("actor '{id}' has {store} but no control type");
        let hint = "give the actor a control type under actors.control_type";
        let diagnostic = schema_error(FILE, &message, hint);
        diagnostics.push(diagnostic.at_key_path(format!("actors.{store}.{id}")));
    }
    for id in actors.query::<ActorId>().iter() {
        if !canon.cast.contains_key(id) {
            let message = format!("actor '{id}' is not a cast member");
            let hint = "add a cast file with this id, or remove the actor from the save";
            diagnostics.push(
                schema_error(FILE, &message, hint).at_key_path(format!("actors.control_type.{id}")),
            );
        }
        if !ids_by_store.contains(&("place", id)) {
            let message = format!("actor '{id}' has a control type but no place");
            let diagnostic = schema_error(FILE, &message, "give every actor a place");
            diagnostics.push(diagnostic.at_key_path(format!("actors.place.{id}")));
        }
    }
    for (id, location) in actors.query::<(ActorId, &component::Place)>().iter() {
        if !location.is_known(canon) {
            let message =
                format!("actor '{id}' is at '{location}', which the pack does not define");
            let diagnostic = schema_error(FILE, &message, "move the actor to a place");
            diagnostics.push(diagnostic.at_key_path(format!("actors.place.{id}")));
        }
    }
    for meter in find_stale_meters(world) {
        let message = format!(
            "lists meter '{}', which the pack does not give it",
            meter.id
        );
        let hint = "run spook migrate to drop the meter";
        diagnostics.push(meter_error(&meter.entity_id, &meter.id, &message, hint));
    }
    for (entity_id, key, meter) in saved_meters(world) {
        let def = canon.meter_def(entity_id.as_ref(), key);
        if def.is_some_and(|def| !meter::is_storable(def, meter.base, meter.offset)) {
            let message = format!("meter '{key}' is outside the rules");
            let hint = "set base and offset so the stored pair fits the meter";
            diagnostics.push(meter_error(&entity_id, key, &message, hint));
        }
    }
    for (id, key, modifier) in find_stale_modifiers(world) {
        let message = format!(
            "actor '{id}' holds modifier '{key}' on meter '{}', which the pack does not define",
            modifier.meter
        );
        let diagnostic = schema_error(FILE, &message, "run spook migrate to drop the modifier");
        diagnostics.push(diagnostic.at_key_path(format!("actors.modifiers.{id}.{key}")));
    }
    let actor_items = actors.query::<&component::Item>();
    let actor_items: Vec<_> = actor_items
        .iter()
        .flat_map(|entry| entry.iter_with_id())
        .collect();
    for &(id, item, count) in &actor_items {
        if item.is_empty() {
            diagnostics.push(
                schema_error(
                    FILE,
                    &format!("actor '{id}' lists an empty item id"),
                    "give the item a non-empty id",
                )
                .at_key_path(format!("actors.items.{id}")),
            );
        }
        if *count == 0 {
            diagnostics.push(
                schema_error(
                    FILE,
                    &format!("actor '{id}' lists item '{item}' with a count of 0"),
                    "omit an item the actor does not hold",
                )
                .at_key_path(format!("actors.items.{id}.{item}")),
            );
        }
    }
    let place_items = world.places().query::<&component::Item>();
    let place_items: Vec<_> = place_items
        .iter()
        .flat_map(|entry| entry.iter_with_id())
        .collect();
    for &(place, item, count) in &place_items {
        if *count == 0 {
            diagnostics.push(
                schema_error(
                    FILE,
                    &format!("place '{place}' lists item '{item}' with a count of 0"),
                    "omit an item that is not there",
                )
                .at_key_path(format!("places.items.{place}.{item}")),
            );
        }
    }
    for (actor_id, item, _) in actor_items {
        if !canon.items.contains_key(item) {
            let path = format!("actors.items.{actor_id}.{item}");
            diagnostics.push(refuse_unknown_item(FILE, item, &path));
        }
    }
    for (actor_id, wielded) in actors.query::<(ActorId, &component::WieldedItem)>().iter() {
        if !canon.items.contains_key(wielded) {
            diagnostics.push(refuse_unknown_item(
                FILE,
                wielded,
                &format!("actors.wielded.{actor_id}"),
            ));
        } else if !canon
            .items
            .get(wielded)
            .is_some_and(|item: &item::Def| item.tags.iter().any(|tag| tag == "weapon"))
        {
            diagnostics.push(
                schema_error(
                    FILE,
                    &format!("actor '{actor_id}' wields '{wielded}', which is not a weapon"),
                    "wield an item tagged weapon, or leave the slot empty",
                )
                .at_key_path(format!("actors.wielded.{actor_id}")),
            );
        } else if !actors.get::<component::Item>(actor_id).contains(wielded) {
            diagnostics.push(
                schema_error(
                    FILE,
                    &format!("actor '{actor_id}' wields '{wielded}' without holding it"),
                    "put the item in items or clear wielded",
                )
                .at_key_path(format!("actors.wielded.{actor_id}")),
            );
        }
    }
    for (place, item, _) in place_items {
        if !canon.items.contains_key(item) {
            let path = format!("places.items.{place}.{item}");
            diagnostics.push(refuse_unknown_item(FILE, item, &path));
        }
    }
    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(diagnostics)
    }
}

/// A save error on meter `key` of `entity_id`, at the meter's key path: who carries it, then
/// `message`.
fn meter_error(entity_id: &entity::Id, key: &str, message: &str, hint: &str) -> Diagnostic {
    let (who, key_path) = match entity_id {
        entity::Id::Actor(id) => (format!("actor '{id}'"), format!("actors.meters.{id}")),
        entity::Id::Place(id) => (entity_id.to_string(), format!("places.meters.{id}")),
        entity::Id::Path(id) => (entity_id.to_string(), format!("paths.{id}")),
        entity::Id::Story(id) => (entity_id.to_string(), format!("stories.meters.{id}")),
        entity::Id::Global => ("the world".to_string(), "world.meters".to_string()),
    };
    let diagnostic = schema_error(FILE, &format!("{who} {message}"), hint);
    diagnostic.at_key_path(format!("{key_path}.{key}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn docks_now_round_trips_through_emitter() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/packs/docks/state/save.yaml");
        let original = fs::read_to_string(&path).unwrap();
        let empty = "globals:
  meters: {}
  flags: []
";
        let with_globals = "globals:
  meters:
    dawn:
      base: 4
      offset: -1
    tide:
      base: 9
      offset: 0
  flags:
    - gate_locked
    - rang
";
        assert!(original.contains(empty));
        for text in [original.clone(), original.replace(empty, with_globals)] {
            let save = parse(&text).unwrap();
            let emitted = emit(&save).unwrap();
            assert_eq!(emitted, text);
            assert_eq!(parse(&emitted).unwrap(), save);
        }
    }

    fn save_text(tag: &str) -> String {
        format!(
            "\
schema: {tag}
world: ex
pack_rev: blake2s:00
rules_rev: {rules}
state_rev: 0
round_index: 0
world_time: 0
last_round_at: \"2026-01-01T00:00:00Z\"
actors: {{}}
",
            rules = crate::store::RULES_REV,
        )
    }

    #[test]
    fn emit_writes_the_save_tag() {
        let mut save = parse(&save_text("save")).unwrap();
        save.schema = concat!("save/", "v0").to_string();
        let emitted = emit(&save).unwrap();
        assert!(emitted.starts_with("schema: save\n"), "{emitted}");
        assert_eq!(parse(&emitted).unwrap().schema, "save");
    }
}
