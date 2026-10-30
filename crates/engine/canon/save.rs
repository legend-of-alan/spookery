//! Save migration and validation against the loaded pack.

use crate::canon::lint::unknown_item;
use crate::canon::{Canon, ItemDef};
use crate::diagnostic::{schema_error, Diagnostic};
use crate::meter::{self, MeterLive};
use crate::model::State;
use crate::store::codec::normalize;
use crate::store::records::AddedMeter;

/// Meters the pack defines that at least one actor lacks, each at its `start`.
pub(crate) fn missing_meters(canon: &Canon, now: &State) -> Vec<AddedMeter> {
    canon
        .meters
        .iter()
        .filter(|(id, _)| {
            now.actors
                .values()
                .any(|actor| !actor.meters().contains_key(*id))
        })
        .map(|(id, def)| AddedMeter {
            id: id.clone(),
            start: def.start,
        })
        .collect()
}

/// Migrate a save: give each actor the `meters` it lacks, at their `start`, then
/// stamp `pack_rev` and `state_rev`. Live migrate and replay both call this.
/// It does not touch the disk.
pub fn migrate_save(
    mut now: State,
    pack_rev: &str,
    state_rev: u64,
    meters: &[AddedMeter],
) -> State {
    for actor in now.actors.values_mut() {
        let live = actor.meters_mut();
        for meter in meters {
            live.entry(meter.id.clone()).or_insert(MeterLive {
                base: meter.start,
                offset: 0,
            });
        }
    }
    now.pack_rev = pack_rev.to_string();
    now.state_rev = state_rev;
    normalize(&mut now);
    now
}

/// Check the save against the pack without changing it. Refuse every meter,
/// item and wielded slot the pack does not define, every meter the pack defines
/// that an actor lacks, every meter outside its rules and every empty id or zero count.
pub fn check_save(canon: &Canon, now: &State) -> Result<(), Vec<Diagnostic>> {
    let mut diags = Vec::new();
    for (id, actor) in &now.actors {
        for key in canon.meters.keys() {
            if actor.meters().contains_key(key) {
                continue;
            }
            diags.push(
                schema_error(
                    "state/save.yaml",
                    &format!("actor '{id}' lacks meter '{key}', which rules/meters.yaml defines"),
                    "run spook migrate to add the meter",
                )
                .at_path(format!("actors.{id}.meters.{key}")),
            );
        }
        for key in actor.meters().keys() {
            if canon.meters.contains_key(key) {
                continue;
            }
            diags.push(
                schema_error(
                    "state/save.yaml",
                    &format!(
                        "actor '{id}' lists meter '{key}', which rules/meters.yaml does not define"
                    ),
                    "remove the meter or add it to the rules",
                )
                .at_path(format!("actors.{id}.meters.{key}")),
            );
        }
        for (key, meter) in actor.meters() {
            let Some(def) = canon.meters.get(key) else {
                continue;
            };
            if meter::is_storable(def, meter.base, meter.offset) {
                continue;
            }
            diags.push(
                schema_error(
                    "state/save.yaml",
                    &format!("actor '{id}' meter '{key}' is outside the rules"),
                    "set base and offset so the stored pair fits the meter",
                )
                .at_path(format!("actors.{id}.meters.{key}")),
            );
        }
    }
    for (id, actor) in &now.actors {
        for (item, count) in actor.inventory() {
            if item.is_empty() {
                diags.push(
                    schema_error(
                        "state/save.yaml",
                        &format!("actor '{id}' lists an empty item id"),
                        "give the item a non-empty id",
                    )
                    .at_path(format!("actors.{id}.inventory")),
                );
            }
            if *count == 0 {
                diags.push(
                    schema_error(
                        "state/save.yaml",
                        &format!("actor '{id}' lists item '{item}' with a count of 0"),
                        "omit an item the actor does not hold",
                    )
                    .at_path(format!("actors.{id}.inventory.{item}")),
                );
            }
        }
    }
    for (place, live) in &now.places {
        for (item, count) in &live.items {
            if *count == 0 {
                diags.push(
                    schema_error(
                        "state/save.yaml",
                        &format!("place '{place}' lists item '{item}' with a count of 0"),
                        "omit an item that is not there",
                    )
                    .at_path(format!("places.{place}.items.{item}")),
                );
            }
        }
    }
    for (actor_id, actor) in &now.actors {
        for item in actor.inventory().keys() {
            if !canon.items.contains_key(item) {
                diags.push(unknown_item(
                    "state/save.yaml",
                    item,
                    &format!("actors.{actor_id}.inventory.{item}"),
                ));
            }
        }
        if let Some(wielded) = actor.wielded() {
            if !canon.items.contains_key(wielded) {
                diags.push(unknown_item(
                    "state/save.yaml",
                    wielded,
                    &format!("actors.{actor_id}.wielded"),
                ));
            } else if !canon
                .items
                .get(wielded)
                .is_some_and(|item: &ItemDef| item.tags.iter().any(|tag| tag == "weapon"))
            {
                diags.push(
                    schema_error(
                        "state/save.yaml",
                        &format!("actor '{actor_id}' wields '{wielded}', which is not a weapon"),
                        "wield an item tagged weapon, or leave the slot empty",
                    )
                    .at_path(format!("actors.{actor_id}.wielded")),
                );
            } else if actor.item_count(wielded) == 0 {
                diags.push(
                    schema_error(
                        "state/save.yaml",
                        &format!("actor '{actor_id}' wields '{wielded}' without holding it"),
                        "put the item in inventory or clear wielded",
                    )
                    .at_path(format!("actors.{actor_id}.wielded")),
                );
            }
        }
    }
    for (place, live) in &now.places {
        for item in live.items.keys() {
            if !canon.items.contains_key(item) {
                diags.push(unknown_item(
                    "state/save.yaml",
                    item,
                    &format!("places.{place}.items.{item}"),
                ));
            }
        }
    }
    if diags.is_empty() {
        Ok(())
    } else {
        Err(diags)
    }
}
