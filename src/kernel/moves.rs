use std::collections::BTreeMap;

use serde::Deserialize;

use crate::canon::Canon;
use crate::error::Reason;

/// A listed move. `row` is set only when the file carries `effects`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MoveDef {
    pub check: Option<String>,
    pub row: Option<ActionRow>,
}

/// One player action. `effects` is a non-empty list of flag or location templates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionRow {
    pub token: MoveToken,
    pub on: MoveOn,
    pub effects: Vec<RowEffect>,
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

/// A flag template, or one location template with no key and no value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowEffect {
    Flag { key: String, value: bool },
    Location,
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

pub fn needs_roll(canon: &Canon, move_name: &str) -> bool {
    canon
        .moves
        .get(move_name)
        .is_some_and(|def| def.check.is_some())
}

/// Check id named by a row. A move without `effects` is not a row.
pub fn row_check(canon: &Canon, move_name: &str) -> Option<String> {
    let def = canon.moves.get(move_name)?;
    if def.row.is_none() {
        return None;
    }
    def.check.clone()
}

fn move_def(entry: MoveEntry) -> Result<(String, MoveDef), Reason> {
    if entry.name.is_empty() {
        return Err(Reason::Schema);
    }
    let started = entry.token.is_some() || entry.on.is_some() || entry.effects.is_some();
    if !started {
        return Ok((
            entry.name,
            MoveDef {
                check: entry.check,
                row: None,
            },
        ));
    }
    if rejects_effects(&entry.name) {
        return Err(Reason::Schema);
    }
    let effects = entry.effects.ok_or(Reason::Schema)?;
    if effects.is_empty() {
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
    let templates = templates_for(token, on, effects)?;
    if entry.name == "travel" && !location_shape(token, on, &templates) {
        return Err(Reason::Schema);
    }
    Ok((
        entry.name,
        MoveDef {
            check: entry.check,
            row: Some(ActionRow {
                token,
                on,
                effects: templates,
            }),
        },
    ))
}

fn templates_for(
    token: MoveToken,
    on: MoveOn,
    effects: Vec<EffectTemplate>,
) -> Result<Vec<RowEffect>, Reason> {
    match (token, on) {
        (MoveToken::None, MoveOn::Speaker) | (MoveToken::Actor, MoveOn::Named) => {
            let mut templates = Vec::with_capacity(effects.len());
            for effect in effects {
                if effect.class != "flag" {
                    return Err(Reason::Schema);
                }
                let Some(key) = effect.key else {
                    return Err(Reason::Schema);
                };
                let Some(value) = effect.value else {
                    return Err(Reason::Schema);
                };
                if key == "busy" {
                    return Err(Reason::Schema);
                }
                templates.push(RowEffect::Flag { key, value });
            }
            Ok(templates)
        }
        (MoveToken::Place, MoveOn::Speaker) => {
            let mut templates = Vec::with_capacity(effects.len());
            for effect in effects {
                if effect.class != "location" || effect.key.is_some() || effect.value.is_some() {
                    return Err(Reason::Schema);
                }
                templates.push(RowEffect::Location);
            }
            Ok(templates)
        }
        _ => Err(Reason::Schema),
    }
}

/// `travel` may carry effects only as a place token, `on: speaker`, and location templates.
fn location_shape(token: MoveToken, on: MoveOn, effects: &[RowEffect]) -> bool {
    token == MoveToken::Place
        && on == MoveOn::Speaker
        && effects
            .iter()
            .all(|effect| matches!(effect, RowEffect::Location))
}

fn rejects_effects(name: &str) -> bool {
    matches!(
        name,
        "say" | "talk" | "wait" | "attempt" | "attack" | "give" | "take"
    )
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
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EffectTemplate {
    class: String,
    #[serde(default)]
    key: Option<String>,
    #[serde(default)]
    value: Option<bool>,
}
