use crate::canon::Canon;
use crate::effect::{Effect, EffectClass};
use crate::error::Reason;
use crate::firewall::allows;
use crate::limits::{world_hour, MINUTES_PER_DAY};
use crate::model::{ActorKind, Intent, Now, RunState, Target, ViewerId};
use crate::moves::{ActionRow, Field, MoveOn, RowEffect, Selector};

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
        return expand_row(canon, now, intent, row, outcome);
    }
    let viewer = ViewerId::from_kind(intent.actor_kind, &intent.actor_id)?;
    let intent_id = intent.id.clone();
    match intent.move_name.as_str() {
        "wait" | "attempt" => Ok(Vec::new()),
        "talk" | "attack" | "give" | "take" => Err(Reason::NotInSpec),
        "travel" => {
            let Target::Place(to) = &intent.target else {
                return Err(Reason::Schema);
            };
            Ok(vec![location_effect(now, intent, to)?])
        }
        "work" => {
            let Some(profile) = canon.profile(&intent.actor_id) else {
                return Ok(Vec::new());
            };
            let Some(work) = &profile.work else {
                return Ok(Vec::new());
            };
            let mut effects = Vec::new();
            for item in &work.needs {
                effects.push(Effect {
                    class: EffectClass::Need {
                        actor_id: intent.actor_id.clone(),
                        key: item.key.clone(),
                        delta: item.delta,
                    },
                    as_viewer: viewer.clone(),
                    intent_id: intent_id.clone(),
                });
            }
            for item in &work.resources {
                effects.push(Effect {
                    class: EffectClass::Resource {
                        actor_id: intent.actor_id.clone(),
                        key: item.key.clone(),
                        delta: item.delta,
                    },
                    as_viewer: viewer.clone(),
                    intent_id: intent_id.clone(),
                });
            }
            Ok(effects)
        }
        "restock" => {
            let sku = intent.args.sku.clone().ok_or(Reason::UnknownKey)?;
            let delta = intent.args.delta.ok_or(Reason::UnknownKey)?;
            let place = now
                .actors
                .get(&intent.actor_id)
                .map(|actor| actor.place().to_string())
                .ok_or(Reason::UnknownKey)?;
            Ok(vec![Effect {
                class: EffectClass::Stock { place, sku, delta },
                as_viewer: viewer,
                intent_id,
            }])
        }
        "rest" => {
            let delta = intent.args.delta.ok_or(Reason::UnknownKey)?;
            Ok(vec![Effect {
                class: EffectClass::Need {
                    actor_id: intent.actor_id.clone(),
                    key: "rest".to_string(),
                    delta,
                },
                as_viewer: viewer,
                intent_id,
            }])
        }
        "drift_relationship" => {
            let Target::Actor(other) = &intent.target else {
                return Err(Reason::Schema);
            };
            let delta = intent.args.delta.ok_or(Reason::UnknownKey)?;
            Ok(vec![Effect {
                class: EffectClass::Relationship {
                    a: intent.actor_id.clone(),
                    b: other.clone(),
                    delta,
                },
                as_viewer: viewer,
                intent_id,
            }])
        }
        "spread_rumor" => {
            let Target::Rumor(rumor_id) = &intent.target else {
                return Err(Reason::Schema);
            };
            let rumor_id = rumor_id.clone();
            let to = intent.args.to.clone().ok_or(Reason::UnknownKey)?;
            let rumor = now
                .rumors
                .iter()
                .find(|rumor| rumor.id == rumor_id)
                .ok_or(Reason::UnknownKey)?;
            Ok(vec![Effect {
                class: EffectClass::Rumor {
                    rumor_id,
                    fact_ids: rumor.fact_ids.clone(),
                    from: intent.actor_id.clone(),
                    to,
                    heat: rumor.heat,
                },
                as_viewer: viewer,
                intent_id,
            }])
        }
        "leave_note" => Ok(vec![Effect {
            class: EffectClass::Message {
                message_id: intent.args.message_id.clone().ok_or(Reason::UnknownKey)?,
                to: intent.args.message_to.clone().unwrap_or_default(),
                place: intent.args.place.clone(),
                fact_ids: intent.args.fact_ids.clone().unwrap_or_default(),
                text: intent.args.text.clone().ok_or(Reason::UnknownKey)?,
            },
            as_viewer: viewer,
            intent_id,
        }]),
        "file_proposal" => Ok(vec![Effect {
            class: EffectClass::Proposal {
                proposal_id: intent.args.proposal_id.clone().ok_or(Reason::UnknownKey)?,
                kind: intent.args.kind.clone().ok_or(Reason::UnknownKey)?,
                target: intent.args.target.clone().unwrap_or_default(),
                summary: intent.args.summary.clone().ok_or(Reason::UnknownKey)?,
            },
            as_viewer: viewer,
            intent_id,
        }]),
        "advance_clock" => {
            let Target::Clock(clock_id) = &intent.target else {
                return Err(Reason::Schema);
            };
            Ok(vec![Effect {
                class: EffectClass::Clock {
                    clock_id: clock_id.clone(),
                    delta: intent.args.delta.unwrap_or(1),
                },
                as_viewer: viewer,
                intent_id,
            }])
        }
        other => {
            if canon.moves.contains_key(other) {
                Err(Reason::NotInSpec)
            } else {
                Err(Reason::UnknownMove)
            }
        }
    }
}

/// One effect per selected template. `effects` always. `success` or `failure` when that outcome was rolled.
/// A flag's `speaker` is the acting actor and `named` is the typed id.
/// A bare location template is the same effect as the travel arm: `to` is the typed place.
/// Selector walks run here, against the `Now` this call sees.
fn expand_row(
    canon: &Canon,
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
        expand_template(canon, now, intent, row, &viewer, template, &mut effects)?;
    }
    Ok(effects)
}

fn expand_template(
    canon: &Canon,
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
            let place = match to {
                Field::Selector(Selector::SchedulePlace) => schedule_place(canon, now, &intent.actor_id)
                    .ok_or(Reason::UnknownKey)?,
                Field::Literal(place) => place.clone(),
                _ => return Err(Reason::Schema),
            };
            effects.push(location_effect(now, intent, &place)?);
        }
        RowEffect::Resource { key, delta } => {
            let actor_id = actor_of(row, intent)?;
            effects.push(wrap(
                viewer,
                &intent.id,
                EffectClass::Resource {
                    actor_id,
                    key: key.clone(),
                    delta: *delta,
                },
            ));
        }
        RowEffect::Stock { place, sku, delta } => {
            let (place, sku, delta) = stock_fields(canon, now, &intent.actor_id, place, sku, delta)?;
            effects.push(wrap(
                viewer,
                &intent.id,
                EffectClass::Stock { place, sku, delta },
            ));
        }
        RowEffect::Need { key, delta } => {
            for class in need_classes(canon, now, intent, row, key.as_deref(), delta)? {
                effects.push(wrap(viewer, &intent.id, class));
            }
        }
        RowEffect::Clock { clock, delta } => {
            let clock_id = match clock {
                Field::Selector(Selector::FirstOwnedClock) => {
                    first_owned_clock(now, &intent.actor_id).ok_or(Reason::UnknownKey)?
                }
                Field::Literal(id) => id.clone(),
                _ => return Err(Reason::Schema),
            };
            effects.push(wrap(
                viewer,
                &intent.id,
                EffectClass::Clock {
                    clock_id,
                    delta: *delta,
                },
            ));
        }
        RowEffect::Message {
            message: Selector::FirstDueNote,
        } => {
            let profile = canon.profile(&intent.actor_id).ok_or(Reason::UnknownKey)?;
            let note = first_due_note(now, profile).ok_or(Reason::UnknownKey)?;
            effects.push(wrap(
                viewer,
                &intent.id,
                EffectClass::Message {
                    message_id: note.id.clone(),
                    to: note.to.clone(),
                    place: Some(note.place.clone()),
                    fact_ids: note.fact_ids.clone(),
                    text: note.text.clone(),
                },
            ));
        }
        RowEffect::Message { .. } => return Err(Reason::Schema),
        RowEffect::Proposal {
            proposal_id,
            kind,
            target,
            summary,
        } => {
            effects.push(wrap(
                viewer,
                &intent.id,
                EffectClass::Proposal {
                    proposal_id: proposal_id.clone(),
                    kind: kind.clone(),
                    target: target.clone(),
                    summary: summary.clone(),
                },
            ));
        }
        RowEffect::Relationship { to, delta } => {
            let (other, delta) = relationship_fields(now, &intent.actor_id, to, delta)?;
            effects.push(wrap(
                viewer,
                &intent.id,
                EffectClass::Relationship {
                    a: intent.actor_id.clone(),
                    b: other,
                    delta,
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
        RowEffect::Rumor { .. } => {
            let class = rumor_class(canon, now, &intent.actor_id).ok_or(Reason::UnknownKey)?;
            effects.push(wrap(viewer, &intent.id, class));
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

fn stock_fields(
    canon: &Canon,
    now: &Now,
    actor_id: &str,
    place: &Field,
    sku: &Field,
    delta: &Field,
) -> Result<(String, String, i64), Reason> {
    let walk = place == &Field::Selector(Selector::Here)
        && sku == &Field::Selector(Selector::FirstRateSku)
        && delta == &Field::Selector(Selector::Rate);
    if walk {
        return stock_walk(canon, now, actor_id).ok_or(Reason::UnknownKey);
    }
    match (place, sku, delta) {
        (Field::Literal(place), Field::Literal(sku), Field::Number(delta)) => {
            Ok((place.clone(), sku.clone(), *delta))
        }
        _ => Err(Reason::Schema),
    }
}

fn stock_walk(canon: &Canon, now: &Now, actor_id: &str) -> Option<(String, String, i64)> {
    let place = now.actors.get(actor_id)?.place().to_string();
    let rates = canon.profile(actor_id)?.stock_rate.get(&place)?;
    for (sku, rate) in rates {
        let stock = now
            .places
            .get(&place)
            .and_then(|place_live| place_live.stock.get(sku))
            .copied()
            .unwrap_or(0);
        if stock.checked_add(*rate).is_some_and(|next| next >= 0) {
            return Some((place, sku.clone(), *rate));
        }
    }
    None
}

fn need_classes(
    canon: &Canon,
    now: &Now,
    intent: &Intent,
    row: &ActionRow,
    key: Option<&str>,
    delta: &Field,
) -> Result<Vec<EffectClass>, Reason> {
    match delta {
        Field::Selector(Selector::FromWork) => Ok(from_work(canon, &intent.actor_id)),
        Field::Selector(Selector::ScheduleRestDelta) => {
            let delta = schedule_rest(canon, now, &intent.actor_id).ok_or(Reason::UnknownKey)?;
            Ok(vec![EffectClass::Need {
                actor_id: intent.actor_id.clone(),
                key: "rest".to_string(),
                delta,
            }])
        }
        Field::Number(delta) => Ok(vec![EffectClass::Need {
            actor_id: actor_of(row, intent)?,
            key: key.unwrap_or("").to_string(),
            delta: *delta,
        }]),
        _ => Err(Reason::Schema),
    }
}

fn from_work(canon: &Canon, actor_id: &str) -> Vec<EffectClass> {
    let Some(work) = canon.profile(actor_id).and_then(|profile| profile.work.as_ref()) else {
        return Vec::new();
    };
    let mut classes = Vec::new();
    for item in &work.needs {
        classes.push(EffectClass::Need {
            actor_id: actor_id.to_string(),
            key: item.key.clone(),
            delta: item.delta,
        });
    }
    for item in &work.resources {
        classes.push(EffectClass::Resource {
            actor_id: actor_id.to_string(),
            key: item.key.clone(),
            delta: item.delta,
        });
    }
    classes
}

fn schedule_rest(canon: &Canon, now: &Now, actor_id: &str) -> Option<i64> {
    let profile = canon.profile(actor_id)?;
    let hour = world_hour(now.world_time);
    profile.winning_row(hour)?.rest_delta
}

fn schedule_place(canon: &Canon, now: &Now, actor_id: &str) -> Option<String> {
    if now.pause.travel != RunState::Running {
        return None;
    }
    let profile = canon.profile(actor_id)?;
    let actor_place = now.actors.get(actor_id)?.place().to_string();
    let hour = world_hour(now.world_time);
    let row = profile.winning_row(hour)?;
    if row.place == actor_place || !profile.range.iter().any(|item| item == &row.place) {
        return None;
    }
    Some(row.place.clone())
}

fn first_owned_clock(now: &Now, actor_id: &str) -> Option<String> {
    let humans = now.presence.humans;
    now.clocks
        .iter()
        .find(|(_, clock)| {
            clock.owners.iter().any(|owner| owner == actor_id)
                && !clock.paused
                && clock.value < clock.max
                && !(clock.requires_presence && humans == 0)
        })
        .map(|(id, _)| id.clone())
}

fn first_due_note<'a>(
    now: &Now,
    profile: &'a crate::canon::TickProfile,
) -> Option<&'a crate::canon::NoteDef> {
    profile.notes.iter().find(|note| {
        now.clocks
            .get(&note.due_clock)
            .is_some_and(|clock| clock.value >= note.due_value)
            && !now.messages.iter().any(|message| message.id == note.id)
    })
}

fn relationship_fields(
    now: &Now,
    actor_id: &str,
    to: &Field,
    delta: &Field,
) -> Result<(String, i64), Reason> {
    if to == &Field::Selector(Selector::FirstDrift) && delta == &Field::Selector(Selector::TowardTarget)
    {
        return first_drift(now, actor_id).ok_or(Reason::UnknownKey);
    }
    match (to, delta) {
        (Field::Literal(other), Field::Number(delta)) => Ok((other.clone(), *delta)),
        _ => Err(Reason::Schema),
    }
}

fn first_drift(now: &Now, actor_id: &str) -> Option<(String, i64)> {
    let actor = now.actors.get(actor_id)?;
    if actor.busy() {
        return None;
    }
    let relations = actor.relationships()?;
    let mut pairs: Vec<(String, i64)> = relations
        .iter()
        .filter(|(other, relation)| {
            relation.stance != relation.target
                && now
                    .world_time
                    .saturating_sub(relation.last_interaction_world_time)
                    >= MINUTES_PER_DAY
                && now
                    .actors
                    .get(*other)
                    .is_some_and(|other_actor| !other_actor.busy())
        })
        .map(|(other, relation)| {
            let delta = (relation.target - relation.stance).clamp(-1, 1);
            (other.clone(), delta)
        })
        .collect();
    pairs.sort_by(|left, right| left.0.cmp(&right.0));
    pairs.into_iter().next()
}

fn rumor_class(canon: &Canon, now: &Now, actor_id: &str) -> Option<EffectClass> {
    if canon.budget.max_rumor_hops == 0 {
        return None;
    }
    let place = now.actors.get(actor_id)?.place().to_string();
    let mut rumors: Vec<&crate::model::Rumor> = now
        .rumors
        .iter()
        .filter(|rumor| rumor.heat >= 1 && rumor.known_by.iter().any(|id| id == actor_id))
        .collect();
    rumors.sort_by(|left, right| left.id.cmp(&right.id));
    for rumor in rumors {
        let mut recipients: Vec<String> = now
            .actors
            .keys()
            .filter(|other| *other != actor_id)
            .filter(|other| adjacent(now, actor_id, other, &place))
            .filter(|other| {
                let kind = match now.actors.get(*other) {
                    Some(actor) if actor.is_npc() => ActorKind::Npc,
                    Some(actor) if actor.is_pc() => ActorKind::Player,
                    _ => return false,
                };
                let Ok(viewer) = ViewerId::from_kind(kind, other) else {
                    return false;
                };
                rumor
                    .fact_ids
                    .iter()
                    .all(|fact| allows(canon, now, &viewer, fact))
            })
            .cloned()
            .collect();
        recipients.sort();
        if let Some(to) = recipients.into_iter().next() {
            return Some(EffectClass::Rumor {
                rumor_id: rumor.id.clone(),
                fact_ids: rumor.fact_ids.clone(),
                from: actor_id.to_string(),
                to,
                heat: rumor.heat,
            });
        }
    }
    None
}

fn adjacent(now: &Now, actor_id: &str, other: &str, place: &str) -> bool {
    let same_place = now
        .actors
        .get(other)
        .is_some_and(|actor| actor.place() == place);
    let related = now
        .actors
        .get(actor_id)
        .and_then(|actor| actor.relationships())
        .is_some_and(|relations| relations.contains_key(other));
    same_place || related
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
    use crate::effect::EffectClass;
    use crate::error::Reason;
    use crate::model::{
        ActorKind, Clock, Intent, IntentArgs, Source, Target, ViewerId,
    };
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
    fn row_emits_stock_need_clock_message_and_proposal_through_apply_effect() {
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
    audience: agenda
    token: none
    on: speaker
    effects:
      - class: stock
        place: here
        sku: first_rate
        delta: rate
      - class: need
        delta: from_work
      - class: clock
        clock: first_owned
        delta: 1
      - class: message
        message: first_due_note
      - class: proposal
        proposal: shelf_note
        kind: other
        target: ""
        summary: counted
  - name: overstock
    token: none
    on: speaker
    effects:
      - class: stock
        place: docks_shop
        sku: lamp_oil
        delta: 5
  - name: overclock
    token: none
    on: speaker
    effects:
      - class: clock
        clock: shelf_count
        delta: 9
  - name: overneed
    token: none
    on: speaker
    effects:
      - class: need
        key: rest
        delta: 9223372036854775807
  - name: bad_proposal
    token: none
    on: speaker
    effects:
      - class: proposal
        proposal: shelf_note
        kind: invent
        target: ""
        summary: no
  - name: hold
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
                EffectClass::Need {
                    actor_id: "mira".to_string(),
                    key: "rest".to_string(),
                    delta: 1,
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
                EffectClass::Proposal {
                    proposal_id: "shelf_note".to_string(),
                    kind: "other".to_string(),
                    target: String::new(),
                    summary: "counted".to_string(),
                },
            ]
        );
        assert!(expanded.iter().all(|effect| {
            effect.as_viewer() == &ViewerId::Npc("mira".to_string()) && effect.intent_id() == "shelf-1"
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
        assert_eq!(txn.now().actors["mira"].needs().unwrap()["rest"], 4);
        assert_eq!(txn.now().clocks["shelf_count"].value, 1);
        assert_eq!(txn.now().messages[0].id, "note_mira_door");
        assert_eq!(txn.now().proposals, vec!["shelf_note".to_string()]);
        assert_eq!(txn.proposals()[0].kind, "other");
        assert_eq!(txn.proposals()[0].actor_id, "mira");

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
            apply_effect(&mut clock_txn, &overclock, clock.into_iter().next().unwrap()).unwrap_err(),
            Reason::OutOfRange
        );
        assert_eq!(clock_txn.now().clocks["shelf_count"].value, 0);

        let mut need_txn = crate::shard::Txn::begin(&shard);
        let overneed = intent("overneed", Target::Empty);
        let need = expand_outcome(shard.canon(), shard.now(), &overneed, None).unwrap();
        assert_eq!(
            apply_effect(&mut need_txn, &overneed, need.into_iter().next().unwrap()).unwrap_err(),
            Reason::OutOfRange
        );
        assert_eq!(need_txn.now().actors["mira"].needs().unwrap()["rest"], 3);

        let mut proposal_txn = crate::shard::Txn::begin(&shard);
        let bad = intent("bad_proposal", Target::Empty);
        let proposal = expand_outcome(shard.canon(), shard.now(), &bad, None).unwrap();
        assert_eq!(
            apply_effect(
                &mut proposal_txn,
                &bad,
                proposal.into_iter().next().unwrap()
            )
            .unwrap_err(),
            Reason::Schema
        );
        assert!(proposal_txn.now().proposals.is_empty());

        let hold = intent("hold", Target::Empty);
        let yielded = expand_outcome(shard.canon(), shard.now(), &hold, None).unwrap();
        assert!(yielded.is_empty());
        let travel = shard.canon().moves.get("travel").unwrap().row.as_ref().unwrap();
        assert!(travel.audience.is_none());
    }
}
