use crate::canon::Canon;
use crate::effect::{Effect, EffectClass};
use crate::error::Reason;
use crate::intent::{Intent, Target, ViewerId};
use crate::meter::WorkWrite;
use crate::model::Now;
use crate::moves::{ActionRow, MoveOn, RowEffect};

pub fn expand(canon: &Canon, now: &Now, intent: &Intent) -> Result<Vec<Effect>, Reason> {
    expand_outcome(canon, now, intent, None)
}

/// `outcome` selects `success` (`Some(true)`) or `failure` (`Some(false)`).
/// `None` means the row did not roll. `effects` is included either way.
pub fn expand_outcome(
    canon: &Canon,
    now: &Now,
    intent: &Intent,
    outcome: Option<bool>,
) -> Result<Vec<Effect>, Reason> {
    if let Some(row) = canon
        .moves
        .get(&intent.move_name)
        .and_then(|def| def.row.as_ref())
    {
        return expand_row(now, intent, row, outcome);
    }
    if canon.moves.contains_key(&intent.move_name) {
        Err(Reason::NotInSpec)
    } else {
        Err(Reason::UnknownMove)
    }
}

/// One effect per selected template. `effects` always. `success` or `failure` when that outcome was rolled.
/// A flag's `speaker` is the acting actor and `named` is the typed id.
/// A bare location template is the same effect as the travel arm: `to` is the typed place.
/// Literals are copied here. A rumor row copies `fact_ids` from the named rumor.
fn expand_row(
    now: &Now,
    intent: &Intent,
    row: &ActionRow,
    outcome: Option<bool>,
) -> Result<Vec<Effect>, Reason> {
    let viewer = ViewerId::from_kind(intent.actor_kind, &intent.actor_id)?;
    let mut selected =
        Vec::with_capacity(row.effects.len() + row.success.len().max(row.failure.len()));
    selected.extend(row.effects.iter());
    match outcome {
        Some(true) => selected.extend(row.success.iter()),
        Some(false) => selected.extend(row.failure.iter()),
        None => {}
    }
    let mut effects = Vec::with_capacity(selected.len());
    for template in selected {
        expand_template(now, intent, row, &viewer, template, &mut effects)?;
    }
    Ok(effects)
}

fn expand_template(
    now: &Now,
    intent: &Intent,
    row: &ActionRow,
    viewer: &ViewerId,
    template: &RowEffect,
    effects: &mut Vec<Effect>,
) -> Result<(), Reason> {
    match template {
        RowEffect::Flag { key, value } => {
            let actor_id = actor_of(row, intent)?;
            effects.push(wrap(
                viewer,
                &intent.id,
                EffectClass::Flag {
                    actor_id,
                    key: key.clone(),
                    value: *value,
                },
            ));
        }
        RowEffect::Location => {
            if row.on != MoveOn::Speaker {
                return Err(Reason::Schema);
            }
            let Target::Place(to) = &intent.target else {
                return Err(Reason::Schema);
            };
            effects.push(location_effect(now, intent, to)?);
        }
        RowEffect::LocationTo { to } => {
            effects.push(location_effect(now, intent, to)?);
        }
        RowEffect::Meter(write) => {
            effects.push(wrap(
                viewer,
                &intent.id,
                meter_class(&actor_of(row, intent)?, write),
            ));
        }
        RowEffect::Stock { place, sku, delta } => {
            effects.push(wrap(
                viewer,
                &intent.id,
                EffectClass::Stock {
                    place: place.clone(),
                    sku: sku.clone(),
                    delta: *delta,
                },
            ));
        }
        RowEffect::Clock { clock, delta } => {
            effects.push(wrap(
                viewer,
                &intent.id,
                EffectClass::Clock {
                    clock_id: clock.clone(),
                    delta: *delta,
                },
            ));
        }
        RowEffect::Message {
            id,
            to,
            place,
            fact_ids,
            text,
        } => {
            effects.push(wrap(
                viewer,
                &intent.id,
                EffectClass::Message {
                    message_id: id.clone(),
                    to: to.clone(),
                    place: place.clone(),
                    fact_ids: fact_ids.clone(),
                    text: text.clone(),
                },
            ));
        }
        RowEffect::Relationship { to, delta } => {
            effects.push(wrap(
                viewer,
                &intent.id,
                EffectClass::Relationship {
                    a: intent.actor_id.clone(),
                    b: to.clone(),
                    delta: *delta,
                },
            ));
        }
        RowEffect::Reputation { faction, delta } => {
            let actor_id = actor_of(row, intent)?;
            effects.push(wrap(
                viewer,
                &intent.id,
                EffectClass::Reputation {
                    actor_id,
                    faction: faction.clone(),
                    delta: *delta,
                },
            ));
        }
        RowEffect::Memory {
            fact_ids,
            tags,
            text,
        } => {
            let actor_id = actor_of(row, intent)?;
            effects.push(wrap(
                viewer,
                &intent.id,
                EffectClass::Memory {
                    actor_id,
                    fact_ids: fact_ids.clone(),
                    tags: tags.clone(),
                    text: text.clone(),
                },
            ));
        }
        RowEffect::Rumor { rumor, to, heat } => {
            let fact_ids = now
                .rumors
                .iter()
                .find(|record| record.id == *rumor)
                .map(|record| record.fact_ids.clone())
                .ok_or(Reason::UnknownKey)?;
            effects.push(wrap(
                viewer,
                &intent.id,
                EffectClass::Rumor {
                    rumor_id: rumor.clone(),
                    fact_ids,
                    from: intent.actor_id.clone(),
                    to: to.clone(),
                    heat: *heat,
                },
            ));
        }
    }
    Ok(())
}

fn wrap(viewer: &ViewerId, intent_id: &str, class: EffectClass) -> Effect {
    Effect {
        class,
        as_viewer: viewer.clone(),
        intent_id: intent_id.to_string(),
    }
}

fn actor_of(row: &ActionRow, intent: &Intent) -> Result<String, Reason> {
    match row.on {
        MoveOn::Speaker => Ok(intent.actor_id.clone()),
        MoveOn::Named => match &intent.target {
            Target::Actor(id) => Ok(id.clone()),
            _ => Err(Reason::Schema),
        },
    }
}

fn meter_class(actor_id: &str, write: &WorkWrite) -> EffectClass {
    EffectClass::Meter {
        actor_id: actor_id.to_string(),
        key: write.key.clone(),
        write: write.write,
    }
}

fn location_effect(now: &Now, intent: &Intent, to: &str) -> Result<Effect, Reason> {
    let viewer = ViewerId::from_kind(intent.actor_kind, &intent.actor_id)?;
    let from = now
        .actors
        .get(&intent.actor_id)
        .map(|actor| actor.place().to_string())
        .ok_or(Reason::UnknownKey)?;
    Ok(Effect {
        class: EffectClass::Location {
            actor_id: intent.actor_id.clone(),
            from,
            to: to.to_string(),
        },
        as_viewer: viewer,
        intent_id: intent.id.clone(),
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::apply::apply_effect;
    use crate::canon::pack_rev;
    use crate::codec::{emit_now, load_now};
    use crate::effect::{Effect, EffectClass};
    use crate::error::Reason;
    use crate::intent::{ActorKind, Intent, IntentArgs, Source, Target, ViewerId};
    use crate::meter::MeterWrite;
    use crate::model::Clock;
    use crate::shard::open_shard;

    use super::expand_outcome;

    struct TempPack(PathBuf);

    impl Drop for TempPack {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn copy_dir(src: &Path, dst: &Path) {
        fs::create_dir_all(dst).unwrap();
        for entry in fs::read_dir(src).unwrap() {
            let entry = entry.unwrap();
            let to = dst.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_dir(&entry.path(), &to);
            } else {
                fs::copy(entry.path(), &to).unwrap();
            }
        }
    }

    fn intent(name: &str, target: Target) -> Intent {
        Intent {
            id: format!("{name}-1"),
            tick_index: 0,
            actor_id: "mira".to_string(),
            actor_kind: ActorKind::Npc,
            source: Source::Kernel,
            move_name: name.to_string(),
            target,
            args: IntentArgs::default(),
            rationale_code: String::new(),
        }
    }

    #[test]
    fn row_emits_stock_need_clock_and_message_through_apply_effect() {
        let docks = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packs/oneshot-docks");
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("spookery-s6-rows-{nanos}"));
        let _pack = TempPack(root.clone());
        copy_dir(&docks, &root);
        assert_ne!(root, docks);
        let moves = root.join("rules/moves.yaml");
        let mut text = fs::read_to_string(&moves).unwrap();
        text.push_str(
            r#"
  - name: shelf
    audience: habit
    token: none
    on: speaker
    effects:
      - class: stock
        place: docks_shop
        sku: lamp_oil
        delta: 4
      - class: meter
        key: rest
        write: base
        delta: 1
      - class: clock
        clock: shelf_count
        delta: 1
      - class: message
        id: note_mira_door
        to:
          - pc_rowan
        place: docks_shop
        fact_ids: []
        text: Back at dusk. Oil on the shelf.
  - name: overstock
    audience: player
    token: none
    on: speaker
    effects:
      - class: stock
        place: docks_shop
        sku: lamp_oil
        delta: 5
  - name: overclock
    audience: player
    token: none
    on: speaker
    effects:
      - class: clock
        clock: shelf_count
        delta: 9
  - name: overneed
    audience: player
    token: none
    on: speaker
    effects:
      - class: meter
        key: rest
        write: base
        delta: 9223372036854775807
  - name: cap
    audience: player
    token: none
    on: speaker
    effects:
      - class: meter
        key: rest
        write: base
        delta: 100
  - name: creep
    audience: player
    token: none
    on: speaker
    effects:
      - class: meter
        key: rest
        write: base
        delta: 1
  - name: hold
    audience: player
    token: none
    on: speaker
    effects: []
"#,
        );
        fs::write(&moves, text).unwrap();
        let now_path = root.join("state/now.yaml");
        let mut now = load_now(&fs::read_to_string(&now_path).unwrap()).unwrap();
        *now.actors.get_mut("mira").unwrap().place_mut() = "docks_shop".to_string();
        now.clocks.insert(
            "shelf_count".to_string(),
            Clock {
                value: 0,
                max: 3,
                requires_presence: false,
                owners: vec!["mira".to_string()],
                paused: false,
            },
        );
        now.pack_rev = pack_rev(&root).unwrap();
        fs::write(&now_path, emit_now(&now).unwrap()).unwrap();

        let shard = open_shard(&root).unwrap();
        let shelf = intent("shelf", Target::Empty);
        let expanded = expand_outcome(shard.canon(), shard.now(), &shelf, None).unwrap();
        assert_eq!(
            expanded
                .iter()
                .map(|effect| effect.class.clone())
                .collect::<Vec<_>>(),
            vec![
                EffectClass::Stock {
                    place: "docks_shop".to_string(),
                    sku: "lamp_oil".to_string(),
                    delta: 4,
                },
                EffectClass::Meter {
                    actor_id: "mira".to_string(),
                    key: "rest".to_string(),
                    write: MeterWrite::Base(1),
                },
                EffectClass::Clock {
                    clock_id: "shelf_count".to_string(),
                    delta: 1,
                },
                EffectClass::Message {
                    message_id: "note_mira_door".to_string(),
                    to: vec!["pc_rowan".to_string()],
                    place: Some("docks_shop".to_string()),
                    fact_ids: Vec::new(),
                    text: "Back at dusk. Oil on the shelf.".to_string(),
                },
            ]
        );
        assert!(expanded.iter().all(|effect| {
            effect.as_viewer() == &ViewerId::Npc("mira".to_string())
                && effect.intent_id() == "shelf-1"
        }));

        let mut txn = crate::shard::Txn::begin(&shard);
        for effect in expanded {
            apply_effect(&mut txn, &shelf, effect).unwrap();
        }
        assert_eq!(
            txn.now().places["docks_shop"].stock["lamp_oil"],
            8,
            "stock range check accepted delta 4 against rate 4"
        );
        let rest = &txn.now().actors["mira"].meters()["rest"];
        assert_eq!(rest.base, 4);
        assert_eq!(rest.offset, 0);
        assert_eq!(txn.now().clocks["shelf_count"].value, 1);
        assert_eq!(txn.now().messages[0].id, "note_mira_door");

        let again = expand_outcome(shard.canon(), shard.now(), &shelf, None).unwrap();
        let message = again
            .into_iter()
            .find(|effect| matches!(effect.class, EffectClass::Message { .. }))
            .unwrap();
        assert_eq!(
            apply_effect(&mut txn, &shelf, message).unwrap_err(),
            Reason::UnknownKey
        );

        let mut ranged = crate::shard::Txn::begin(&shard);
        let overstock = intent("overstock", Target::Empty);
        let stock = expand_outcome(shard.canon(), shard.now(), &overstock, None).unwrap();
        assert_eq!(stock.len(), 1);
        assert_eq!(
            apply_effect(&mut ranged, &overstock, stock.into_iter().next().unwrap()).unwrap_err(),
            Reason::OutOfRange
        );
        assert_eq!(ranged.now().places["docks_shop"].stock["lamp_oil"], 4);

        let mut clock_txn = crate::shard::Txn::begin(&shard);
        let overclock = intent("overclock", Target::Empty);
        let clock = expand_outcome(shard.canon(), shard.now(), &overclock, None).unwrap();
        assert_eq!(
            apply_effect(
                &mut clock_txn,
                &overclock,
                clock.into_iter().next().unwrap()
            )
            .unwrap_err(),
            Reason::OutOfRange
        );
        assert_eq!(clock_txn.now().clocks["shelf_count"].value, 0);

        let mut stranger_txn = crate::shard::Txn::begin(&shard);
        let stranger = Intent {
            id: "bram-clock".to_string(),
            tick_index: 0,
            actor_id: "bram".to_string(),
            actor_kind: ActorKind::Npc,
            source: Source::Kernel,
            move_name: "overclock".to_string(),
            target: Target::Empty,
            args: IntentArgs::default(),
            rationale_code: String::new(),
        };
        let borrowed = Effect {
            class: EffectClass::Clock {
                clock_id: "shelf_count".to_string(),
                delta: 1,
            },
            as_viewer: ViewerId::Npc("bram".to_string()),
            intent_id: "bram-clock".to_string(),
        };
        assert_eq!(
            apply_effect(&mut stranger_txn, &stranger, borrowed).unwrap_err(),
            Reason::UnknownKey
        );
        assert_eq!(stranger_txn.now().clocks["shelf_count"].value, 0);

        let mut need_txn = crate::shard::Txn::begin(&shard);
        let overneed = intent("overneed", Target::Empty);
        let need = expand_outcome(shard.canon(), shard.now(), &overneed, None).unwrap();
        assert_eq!(
            apply_effect(&mut need_txn, &overneed, need.into_iter().next().unwrap()).unwrap_err(),
            Reason::OutOfRange
        );
        let held = &need_txn.now().actors["mira"].meters()["rest"];
        assert_eq!(held.base, 3);
        assert_eq!(held.offset, 0);

        let mut cap_txn = crate::shard::Txn::begin(&shard);
        let cap = intent("cap", Target::Empty);
        let capped = expand_outcome(shard.canon(), shard.now(), &cap, None).unwrap();
        apply_effect(&mut cap_txn, &cap, capped.into_iter().next().unwrap()).unwrap();
        let capped_rest = &cap_txn.now().actors["mira"].meters()["rest"];
        assert_eq!(capped_rest.base, 5);
        assert_eq!(capped_rest.offset, 0);
        assert!(cap_txn
            .staged_paths()
            .iter()
            .any(|line| line == "actor_meter mira rest"));

        let mut creep_txn = crate::shard::Txn::begin(&shard);
        {
            let live = creep_txn
                .now_mut()
                .actors
                .get_mut("mira")
                .unwrap()
                .meters_mut()
                .get_mut("rest")
                .unwrap();
            live.base = 5;
            live.offset = 0;
        }
        let creep = intent("creep", Target::Empty);
        let crept = expand_outcome(shard.canon(), shard.now(), &creep, None).unwrap();
        apply_effect(&mut creep_txn, &creep, crept.into_iter().next().unwrap()).unwrap();
        let crept_rest = &creep_txn.now().actors["mira"].meters()["rest"];
        assert_eq!(crept_rest.base, 5);
        assert_eq!(crept_rest.offset, 0);
        assert!(creep_txn
            .staged_paths()
            .iter()
            .all(|line| line != "actor_meter mira rest"));

        let hold = intent("hold", Target::Empty);
        let yielded = expand_outcome(shard.canon(), shard.now(), &hold, None).unwrap();
        assert!(yielded.is_empty());
        let travel = shard
            .canon()
            .moves
            .get("travel")
            .unwrap()
            .row
            .as_ref()
            .unwrap();
        assert_eq!(travel.audience, Some(crate::moves::Audience::Player));
    }
}
