use std::collections::BTreeMap;

use serde::Deserialize;

use crate::error::Reason;
use crate::meter::{MeterWrite, WorkWrite};

/// A listed move. `row` is set when the file carries a template list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MoveDef {
    pub check: Option<String>,
    pub row: Option<ActionRow>,
}

/// One action row. `effects` runs on both outcomes. `success` and `failure` run on that outcome.
/// An empty `effects` list with neither outcome list is a yield.
/// A row's `audience` is required. `item`, when set, must already be on the actor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionRow {
    pub token: MoveToken,
    pub on: MoveOn,
    pub effects: Vec<RowEffect>,
    pub success: Vec<RowEffect>,
    pub failure: Vec<RowEffect>,
    pub item: Option<String>,
    pub audience: Option<Audience>,
    pub difficulty: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Audience {
    Player,
    Habit,
    System,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveToken {
    None,
    Actor,
    Place,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveOn {
    Speaker,
    Named,
}

/// One effect template. Flag and location keep the shapes the harness compares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowEffect {
    Flag {
        key: String,
        value: bool,
    },
    Location,
    LocationTo {
        to: String,
    },
    Meter(WorkWrite),
    Stock {
        place: String,
        sku: String,
        delta: i64,
    },
    Clock {
        clock: String,
        delta: i64,
    },
    Message {
        id: String,
        to: Vec<String>,
        place: Option<String>,
        fact_ids: Vec<String>,
        text: String,
    },
    Relationship {
        to: String,
        delta: i64,
    },
    Reputation {
        faction: String,
        delta: i64,
    },
    Memory {
        fact_ids: Vec<String>,
        tags: Vec<String>,
        text: Option<String>,
    },
    Rumor {
        rumor: String,
        to: String,
        heat: i64,
    },
}

pub fn load_moves(text: &str) -> Result<BTreeMap<String, MoveDef>, Reason> {
    let file: MovesFile = serde_saphyr::from_str(text).map_err(|_| Reason::Schema)?;
    if file.schema != "moves/v0" {
        return Err(Reason::Schema);
    }
    let mut moves = BTreeMap::new();
    for entry in file.moves {
        let (name, def) = move_def(entry)?;
        if moves.insert(name, def).is_some() {
            return Err(Reason::Schema);
        }
    }
    Ok(moves)
}

fn move_def(entry: MoveEntry) -> Result<(String, MoveDef), Reason> {
    if entry.name.is_empty() {
        return Err(Reason::Schema);
    }
    if entry.item.as_ref().is_some_and(|id| id.is_empty()) {
        return Err(Reason::Schema);
    }
    let started = entry.token.is_some()
        || entry.on.is_some()
        || entry.effects.is_some()
        || entry.success.is_some()
        || entry.failure.is_some()
        || entry.item.is_some()
        || entry.audience.is_some()
        || entry.difficulty.is_some();
    if !started {
        return Ok((
            entry.name,
            MoveDef {
                check: entry.check,
                row: None,
            },
        ));
    }
    if forbids_row(&entry.name) {
        return Err(Reason::Schema);
    }
    let audience = match entry.audience.as_deref() {
        Some("player") => Audience::Player,
        Some("habit") => Audience::Habit,
        Some("system") => Audience::System,
        _ => return Err(Reason::Schema),
    };
    // `effects: []` with neither outcome list is a yield. An empty success or failure list is not.
    let yield_row = entry.effects.as_ref().is_some_and(Vec::is_empty)
        && entry.success.is_none()
        && entry.failure.is_none();
    if entry.effects.as_ref().is_some_and(Vec::is_empty) && !yield_row {
        return Err(Reason::Schema);
    }
    if entry.success.as_ref().is_some_and(Vec::is_empty)
        || entry.failure.as_ref().is_some_and(Vec::is_empty)
    {
        return Err(Reason::Schema);
    }
    let token = match entry.token.as_deref() {
        Some("none") => MoveToken::None,
        Some("actor") => MoveToken::Actor,
        Some("place") => MoveToken::Place,
        _ => return Err(Reason::Schema),
    };
    let on = match entry.on.as_deref() {
        Some("speaker") => MoveOn::Speaker,
        Some("named") => MoveOn::Named,
        _ => return Err(Reason::Schema),
    };
    let effects = match entry.effects {
        Some(items) if !items.is_empty() => templates_for(token, on, items)?,
        _ => Vec::new(),
    };
    let success = match entry.success {
        Some(items) => outcome_templates(token, on, items)?,
        None => Vec::new(),
    };
    let failure = match entry.failure {
        Some(items) => outcome_templates(token, on, items)?,
        None => Vec::new(),
    };
    if !yield_row && effects.is_empty() && success.is_empty() && failure.is_empty() {
        return Err(Reason::Schema);
    }
    Ok((
        entry.name,
        MoveDef {
            check: entry.check,
            row: Some(ActionRow {
                token,
                on,
                effects,
                success,
                failure,
                item: entry.item,
                audience: Some(audience),
                difficulty: entry.difficulty.unwrap_or(0),
            }),
        },
    ))
}

fn templates_for(
    token: MoveToken,
    on: MoveOn,
    effects: Vec<EffectTemplate>,
) -> Result<Vec<RowEffect>, Reason> {
    let mut templates = Vec::with_capacity(effects.len());
    for effect in effects {
        templates.push(one_template(token, on, effect)?);
    }
    Ok(templates)
}

/// A place token still has neither outcome list.
fn outcome_templates(
    token: MoveToken,
    on: MoveOn,
    effects: Vec<EffectTemplate>,
) -> Result<Vec<RowEffect>, Reason> {
    match (token, on) {
        (MoveToken::None, MoveOn::Speaker) | (MoveToken::Actor, MoveOn::Named) => {
            templates_for(token, on, effects)
        }
        _ => Err(Reason::Schema),
    }
}

fn one_template(token: MoveToken, on: MoveOn, effect: EffectTemplate) -> Result<RowEffect, Reason> {
    if !pair_ok(token, on) {
        return Err(Reason::Schema);
    }
    match effect.class.as_str() {
        "flag" => flag_template(token, on, &effect),
        "location" => location_template(token, &effect),
        "meter" => meter_template(&effect),
        "stock" => stock_template(&effect),
        "clock" => clock_template(&effect),
        "message" => message_template(&effect),
        "relationship" => relationship_template(&effect),
        "reputation" => reputation_template(&effect),
        "memory" => memory_template(&effect),
        "rumor" => rumor_template(&effect),
        _ => Err(Reason::Schema),
    }
}

fn pair_ok(token: MoveToken, on: MoveOn) -> bool {
    matches!(
        (token, on),
        (MoveToken::None, MoveOn::Speaker)
            | (MoveToken::Actor, MoveOn::Named)
            | (MoveToken::Place, MoveOn::Speaker)
    )
}

fn flag_template(
    token: MoveToken,
    on: MoveOn,
    effect: &EffectTemplate,
) -> Result<RowEffect, Reason> {
    only(effect, &["key", "value"])?;
    let flag_pair = matches!(
        (token, on),
        (MoveToken::None, MoveOn::Speaker) | (MoveToken::Actor, MoveOn::Named)
    );
    if !flag_pair {
        return Err(Reason::Schema);
    }
    let Some(key) = effect.key.clone() else {
        return Err(Reason::Schema);
    };
    let Some(value) = effect.value else {
        return Err(Reason::Schema);
    };
    if key == "busy" {
        return Err(Reason::Schema);
    }
    Ok(RowEffect::Flag { key, value })
}

fn location_template(token: MoveToken, effect: &EffectTemplate) -> Result<RowEffect, Reason> {
    only(effect, &["to"])?;
    match effect.to.as_ref() {
        None if token == MoveToken::Place => Ok(RowEffect::Location),
        Some(ToField::One(word)) if word == "token" => {
            if token != MoveToken::Place {
                return Err(Reason::Schema);
            }
            Ok(RowEffect::Location)
        }
        Some(ToField::One(word)) => Ok(RowEffect::LocationTo {
            to: literal_id(Some(word))?,
        }),
        _ => Err(Reason::Schema),
    }
}

fn meter_template(effect: &EffectTemplate) -> Result<RowEffect, Reason> {
    only(effect, &["key", "write", "delta"])?;
    let key = effect.key.clone().ok_or(Reason::Schema)?;
    let write = match (effect.write.as_deref(), effect.delta.as_ref()) {
        (Some("base"), Some(DeltaField::Int(delta))) => MeterWrite::Base(*delta),
        (Some("offset"), Some(DeltaField::Int(delta))) => MeterWrite::Offset(*delta),
        (Some("snap"), None) => MeterWrite::Snap,
        _ => return Err(Reason::Schema),
    };
    Ok(RowEffect::Meter(WorkWrite { key, write }))
}

fn stock_template(effect: &EffectTemplate) -> Result<RowEffect, Reason> {
    only(effect, &["place", "sku", "delta"])?;
    Ok(RowEffect::Stock {
        place: literal_id(effect.place.as_deref())?,
        sku: literal_id(effect.sku.as_deref())?,
        delta: required_number(effect.delta.as_ref())?,
    })
}

fn clock_template(effect: &EffectTemplate) -> Result<RowEffect, Reason> {
    only(effect, &["clock", "delta"])?;
    Ok(RowEffect::Clock {
        clock: literal_id(effect.clock.as_deref())?,
        delta: required_number(effect.delta.as_ref())?,
    })
}

fn message_template(effect: &EffectTemplate) -> Result<RowEffect, Reason> {
    only(effect, &["id", "to", "place", "fact_ids", "text"])?;
    let id = literal_id(effect.id.as_deref())?;
    let to = match effect.to.as_ref() {
        Some(ToField::Many(ids)) if !ids.is_empty() => ids
            .iter()
            .map(|id| literal_id(Some(id.as_str())))
            .collect::<Result<Vec<_>, _>>()?,
        _ => return Err(Reason::Schema),
    };
    let place = match effect.place.as_deref() {
        None => None,
        Some(place) => Some(literal_id(Some(place))?),
    };
    let fact_ids = effect
        .fact_ids
        .iter()
        .flatten()
        .map(|id| literal_id(Some(id.as_str())))
        .collect::<Result<Vec<_>, _>>()?;
    let text = effect.text.clone().ok_or(Reason::Schema)?;
    Ok(RowEffect::Message {
        id,
        to,
        place,
        fact_ids,
        text,
    })
}

fn relationship_template(effect: &EffectTemplate) -> Result<RowEffect, Reason> {
    only(effect, &["to", "delta"])?;
    let to = match effect.to.as_ref() {
        Some(ToField::One(word)) => literal_id(Some(word))?,
        _ => return Err(Reason::Schema),
    };
    Ok(RowEffect::Relationship {
        to,
        delta: required_number(effect.delta.as_ref())?,
    })
}

fn reputation_template(effect: &EffectTemplate) -> Result<RowEffect, Reason> {
    only(effect, &["faction", "delta"])?;
    let delta = required_number(effect.delta.as_ref())?;
    Ok(RowEffect::Reputation {
        faction: effect.faction.clone().ok_or(Reason::Schema)?,
        delta,
    })
}

fn memory_template(effect: &EffectTemplate) -> Result<RowEffect, Reason> {
    only(effect, &["fact_ids", "tags", "text"])?;
    Ok(RowEffect::Memory {
        fact_ids: effect.fact_ids.clone().ok_or(Reason::Schema)?,
        tags: effect.tags.clone().ok_or(Reason::Schema)?,
        text: effect.text.clone(),
    })
}

fn rumor_template(effect: &EffectTemplate) -> Result<RowEffect, Reason> {
    only(effect, &["rumor", "to", "heat"])?;
    let rumor = literal_id(effect.rumor.as_deref())?;
    let to = match effect.to.as_ref() {
        Some(ToField::One(word)) => literal_id(Some(word))?,
        _ => return Err(Reason::Schema),
    };
    let heat = effect.heat.ok_or(Reason::Schema)?;
    Ok(RowEffect::Rumor { rumor, to, heat })
}

fn only(effect: &EffectTemplate, allowed: &[&str]) -> Result<(), Reason> {
    let present = [
        ("key", effect.key.is_some()),
        ("write", effect.write.is_some()),
        ("value", effect.value.is_some()),
        ("delta", effect.delta.is_some()),
        ("to", effect.to.is_some()),
        ("place", effect.place.is_some()),
        ("sku", effect.sku.is_some()),
        ("clock", effect.clock.is_some()),
        ("id", effect.id.is_some()),
        ("faction", effect.faction.is_some()),
        ("fact_ids", effect.fact_ids.is_some()),
        ("tags", effect.tags.is_some()),
        ("text", effect.text.is_some()),
        ("rumor", effect.rumor.is_some()),
        ("heat", effect.heat.is_some()),
    ];
    if present
        .into_iter()
        .any(|(name, set)| set && !allowed.contains(&name))
    {
        return Err(Reason::Schema);
    }
    Ok(())
}

fn literal_id(word: Option<&str>) -> Result<String, Reason> {
    let Some(word) = word else {
        return Err(Reason::Schema);
    };
    if word.is_empty() || word == "token" || retired_word(word) {
        return Err(Reason::Schema);
    }
    Ok(word.to_string())
}

fn retired_word(word: &str) -> bool {
    matches!(
        word,
        "here"
            | "first_rate"
            | "rate"
            | "first_drift"
            | "toward_target"
            | "first_known"
            | "first_listener"
            | "keep"
            | "first_due_note"
            | "first_owned"
    )
}

fn required_number(delta: Option<&DeltaField>) -> Result<i64, Reason> {
    match delta {
        Some(DeltaField::Int(n)) => Ok(*n),
        _ => Err(Reason::Schema),
    }
}

fn forbids_row(name: &str) -> bool {
    name == "say"
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MovesFile {
    schema: String,
    moves: Vec<MoveEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MoveEntry {
    name: String,
    #[serde(default)]
    check: Option<String>,
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    on: Option<String>,
    #[serde(default)]
    effects: Option<Vec<EffectTemplate>>,
    #[serde(default)]
    success: Option<Vec<EffectTemplate>>,
    #[serde(default)]
    failure: Option<Vec<EffectTemplate>>,
    #[serde(default)]
    item: Option<String>,
    #[serde(default)]
    audience: Option<String>,
    #[serde(default)]
    difficulty: Option<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EffectTemplate {
    class: String,
    #[serde(default)]
    key: Option<String>,
    #[serde(default)]
    write: Option<String>,
    #[serde(default)]
    value: Option<bool>,
    #[serde(default)]
    delta: Option<DeltaField>,
    #[serde(default)]
    to: Option<ToField>,
    #[serde(default)]
    place: Option<String>,
    #[serde(default)]
    sku: Option<String>,
    #[serde(default)]
    clock: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    faction: Option<String>,
    #[serde(default)]
    fact_ids: Option<Vec<String>>,
    #[serde(default)]
    tags: Option<Vec<String>>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    rumor: Option<String>,
    #[serde(default)]
    heat: Option<i64>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ToField {
    One(String),
    Many(Vec<String>),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum DeltaField {
    Int(i64),
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::{load_moves, Audience, MoveOn, MoveToken, RowEffect};
    use crate::error::Reason;
    use crate::meter::{MeterWrite, WorkWrite};

    fn schema(body: &str) -> Reason {
        let text = format!("schema: moves/v0\nmoves:\n{body}");
        load_moves(&text).expect_err("schema")
    }

    #[test]
    fn docks_pack_rows_name_an_audience() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packs/oneshot-docks");
        let text = fs::read_to_string(root.join("rules/moves.yaml")).unwrap();
        let moves = load_moves(&text).unwrap();
        let travel = moves.get("travel").unwrap().row.as_ref().unwrap();
        assert_eq!(travel.audience, Some(Audience::Player));
        assert_eq!(travel.difficulty, 0);
        assert_eq!(travel.effects, vec![RowEffect::Location]);
        let wait = moves.get("wait").unwrap().row.as_ref().unwrap();
        assert_eq!(wait.audience, Some(Audience::Player));
        assert!(wait.effects.is_empty());
        let grit = moves.get("grit").unwrap().row.as_ref().unwrap();
        assert_eq!(grit.audience, Some(Audience::Player));
        assert_eq!(moves.get("grit").unwrap().check.as_deref(), Some("grit"));
        assert_eq!(
            moves
                .get("restock_lamp_oil")
                .unwrap()
                .row
                .as_ref()
                .unwrap()
                .audience,
            Some(Audience::Habit)
        );
        assert_eq!(
            moves
                .get("leave_door_note")
                .unwrap()
                .row
                .as_ref()
                .unwrap()
                .audience,
            Some(Audience::Habit)
        );
        assert!(!moves.contains_key("talk"));
        assert!(!moves.contains_key("attempt"));
        assert!(!moves.contains_key("attack"));
        assert!(!moves.contains_key("give"));
        assert!(!moves.contains_key("take"));
        crate::canon::load_canon(&root).unwrap();
    }

    #[test]
    fn a_row_names_ten_classes_literals_and_a_yield() {
        let moves = load_moves(
            r#"schema: moves/v0
moves:
  - name: hold
    audience: player
    token: none
    on: speaker
    effects: []
  - name: shelf
    audience: habit
    token: none
    on: speaker
    difficulty: 2
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
        clock: shop_day
        delta: 1
      - class: message
        id: note_mira_door
        to:
          - pc_rowan
        place: docks_shop
        fact_ids: []
        text: Back at dusk.
      - class: relationship
        to: bram
        delta: -1
      - class: rumor
        rumor: r_184
        to: aldric
        heat: 1
      - class: reputation
        faction: harbor
        delta: 1
      - class: memory
        fact_ids: []
        tags: []
      - class: flag
        key: marked
        value: true
  - name: night
    audience: system
    token: none
    on: speaker
    effects:
      - class: location
        to: docks_shop
      - class: meter
        key: rest
        write: base
        delta: 1
  - name: step
    audience: player
    token: place
    on: speaker
    effects:
      - class: location
        to: token
  - name: mark
    audience: player
    token: actor
    on: named
    success:
      - class: meter
        key: hp
        write: base
        delta: -1
    failure:
      - class: flag
        key: missed
        value: true
"#,
        )
        .unwrap();
        let hold = moves.get("hold").unwrap().row.as_ref().unwrap();
        assert_eq!(hold.audience, Some(Audience::Player));
        assert!(hold.effects.is_empty());
        assert_eq!(hold.difficulty, 0);
        let shelf = moves.get("shelf").unwrap().row.as_ref().unwrap();
        assert_eq!(shelf.audience, Some(Audience::Habit));
        assert_eq!(shelf.difficulty, 2);
        assert_eq!(
            shelf.effects[0],
            RowEffect::Stock {
                place: "docks_shop".to_string(),
                sku: "lamp_oil".to_string(),
                delta: 4,
            }
        );
        assert_eq!(
            shelf.effects[3],
            RowEffect::Message {
                id: "note_mira_door".to_string(),
                to: vec!["pc_rowan".to_string()],
                place: Some("docks_shop".to_string()),
                fact_ids: Vec::new(),
                text: "Back at dusk.".to_string(),
            }
        );
        assert_eq!(
            shelf.effects[4],
            RowEffect::Relationship {
                to: "bram".to_string(),
                delta: -1,
            }
        );
        assert_eq!(
            shelf.effects[5],
            RowEffect::Rumor {
                rumor: "r_184".to_string(),
                to: "aldric".to_string(),
                heat: 1,
            }
        );
        assert_eq!(
            shelf.effects[1],
            RowEffect::Meter(WorkWrite {
                key: "rest".to_string(),
                write: MeterWrite::Base(1),
            })
        );
        let night = moves.get("night").unwrap().row.as_ref().unwrap();
        assert_eq!(night.audience, Some(Audience::System));
        assert_eq!(
            night.effects[0],
            RowEffect::LocationTo {
                to: "docks_shop".to_string(),
            }
        );
        let step = moves.get("step").unwrap().row.as_ref().unwrap();
        assert_eq!(step.audience, Some(Audience::Player));
        assert_eq!(step.effects, vec![RowEffect::Location]);
        let mark = moves.get("mark").unwrap().row.as_ref().unwrap();
        assert!(mark.effects.is_empty());
        assert_eq!(
            night.effects[1],
            RowEffect::Meter(WorkWrite {
                key: "rest".to_string(),
                write: MeterWrite::Base(1),
            })
        );
        assert_eq!(
            mark.success,
            vec![RowEffect::Meter(WorkWrite {
                key: "hp".to_string(),
                write: MeterWrite::Base(-1),
            })]
        );
        assert_eq!(
            mark.failure,
            vec![RowEffect::Flag {
                key: "missed".to_string(),
                value: true,
            }]
        );
        assert_eq!(mark.token, MoveToken::Actor);
        assert_eq!(mark.on, MoveOn::Named);
    }

    #[test]
    fn a_row_named_talk_loads() {
        let moves = load_moves(
            r#"schema: moves/v0
moves:
  - name: talk
    audience: player
    token: none
    on: speaker
    effects: []
"#,
        )
        .unwrap();
        assert!(moves.get("talk").unwrap().row.is_some());
    }

    #[test]
    fn loader_still_rejects_the_old_shapes_and_unknown_selectors() {
        assert_eq!(
            schema(
                "  - name: say\n    audience: player\n    token: none\n    on: speaker\n    effects: []\n"
            ),
            Reason::Schema
        );
        assert_eq!(
            schema("  - name: wait\n    token: none\n    on: speaker\n    effects: []\n"),
            Reason::Schema
        );
        assert_eq!(
            schema(
                "  - name: attack\n    token: actor\n    on: named\n    effects:\n      - class: flag\n        key: marked\n        value: true\n"
            ),
            Reason::Schema
        );
        assert_eq!(
            schema(
                "  - name: travel\n    token: actor\n    on: named\n    effects:\n      - class: flag\n        key: marked\n        value: true\n"
            ),
            Reason::Schema
        );
        assert_eq!(
            schema(
                "  - name: mark\n    token: actor\n    on: named\n    effects: []\n    success:\n      - class: flag\n        key: marked\n        value: true\n"
            ),
            Reason::Schema
        );
        assert_eq!(
            schema(
                "  - name: mark\n    token: actor\n    on: named\n    effects:\n      - class: flag\n        key: marked\n        value: true\n    success: []\n"
            ),
            Reason::Schema
        );
        assert_eq!(
            schema(
                "  - name: mark\n    audience: gm\n    token: none\n    on: speaker\n    effects: []\n"
            ),
            Reason::Schema
        );
        assert_eq!(
            schema(
                "  - name: mark\n    token: none\n    on: speaker\n    effects:\n      - class: inventory\n        key: bag\n"
            ),
            Reason::Schema
        );
        assert_eq!(
            schema(
                "  - name: mark\n    audience: player\n    token: none\n    on: speaker\n    effects:\n      - class: need\n        key: rest\n        delta: 1\n"
            ),
            Reason::Schema
        );
        assert_eq!(
            schema(
                "  - name: mark\n    audience: player\n    token: none\n    on: speaker\n    effects:\n      - class: resource\n        key: rest\n        delta: 1\n"
            ),
            Reason::Schema
        );
        assert_eq!(
            schema(
                "  - name: mark\n    audience: habit\n    token: none\n    on: speaker\n    effects:\n      - class: stock\n        place: here\n        sku: lamp_oil\n        delta: 4\n"
            ),
            Reason::Schema
        );
        assert_eq!(
            schema(
                "  - name: mark\n    audience: habit\n    token: none\n    on: speaker\n    effects:\n      - class: stock\n        place: docks_shop\n        sku: first_rate\n        delta: 4\n"
            ),
            Reason::Schema
        );
        assert_eq!(
            schema(
                "  - name: mark\n    audience: habit\n    token: none\n    on: speaker\n    effects:\n      - class: relationship\n        to: first_drift\n        delta: -1\n"
            ),
            Reason::Schema
        );
        assert_eq!(
            schema(
                "  - name: mark\n    audience: habit\n    token: none\n    on: speaker\n    effects:\n      - class: rumor\n        rumor: first_known\n        to: aldric\n        heat: 1\n"
            ),
            Reason::Schema
        );
        assert_eq!(
            schema(
                "  - name: mark\n    audience: habit\n    token: none\n    on: speaker\n    effects:\n      - class: rumor\n        rumor: r_184\n        to: aldric\n        heat: keep\n"
            ),
            Reason::Schema
        );
        assert_eq!(
            schema(
                "  - name: mark\n    audience: habit\n    token: none\n    on: speaker\n    effects:\n      - class: clock\n        clock: first_owned\n        delta: 1\n"
            ),
            Reason::Schema
        );
        assert_eq!(
            schema(
                "  - name: mark\n    audience: agenda\n    token: none\n    on: speaker\n    effects: []\n"
            ),
            Reason::Schema
        );
        assert_eq!(
            schema(
                "  - name: mark\n    audience: habit\n    token: none\n    on: speaker\n    effects:\n      - class: message\n        message: first_due_note\n"
            ),
            Reason::Schema
        );
        assert_eq!(
            schema(
                "  - name: mark\n    audience: habit\n    token: none\n    on: speaker\n    effects:\n      - class: message\n        id: note\n        to:\n          - first_listener\n        text: hi\n"
            ),
            Reason::Schema
        );
        assert_eq!(
            schema(
                "  - name: mark\n    audience: habit\n    token: none\n    on: speaker\n    effects:\n      - class: message\n        id: note\n        to:\n          - pc_rowan\n        fact_ids:\n          - here\n        text: hi\n"
            ),
            Reason::Schema
        );
        assert_eq!(
            schema(
                "  - name: mark\n    token: none\n    on: speaker\n    effects:\n      - class: location\n        to: token\n"
            ),
            Reason::Schema
        );
        assert_eq!(
            schema(
                "  - name: mark\n    token: none\n    on: speaker\n    effects:\n      - class: flag\n        key: busy\n        value: true\n"
            ),
            Reason::Schema
        );
    }
}
