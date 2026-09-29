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

/// One player action. `effects` is a non-empty list of flag templates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionRow {
    pub token: MoveToken,
    pub on: MoveOn,
    pub effects: Vec<FlagTemplate>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveToken {
    None,
    Actor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveOn {
    Speaker,
    Named,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlagTemplate {
    pub key: String,
    pub value: bool,
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
    if entry.check.is_some() || rejects_effects(&entry.name) {
        return Err(Reason::Schema);
    }
    let effects = entry.effects.ok_or(Reason::Schema)?;
    if effects.is_empty() {
        return Err(Reason::Schema);
    }
    let token = match entry.token.as_deref() {
        Some("none") => MoveToken::None,
        Some("actor") => MoveToken::Actor,
        _ => return Err(Reason::Schema),
    };
    let on = match entry.on.as_deref() {
        Some("speaker") => MoveOn::Speaker,
        Some("named") => MoveOn::Named,
        _ => return Err(Reason::Schema),
    };
    match (token, on) {
        (MoveToken::None, MoveOn::Speaker) | (MoveToken::Actor, MoveOn::Named) => {}
        _ => return Err(Reason::Schema),
    }
    let mut templates = Vec::with_capacity(effects.len());
    for effect in effects {
        if effect.class != "flag" || effect.key == "busy" {
            return Err(Reason::Schema);
        }
        templates.push(FlagTemplate {
            key: effect.key,
            value: effect.value,
        });
    }
    Ok((
        entry.name,
        MoveDef {
            check: None,
            row: Some(ActionRow {
                token,
                on,
                effects: templates,
            }),
        },
    ))
}

fn rejects_effects(name: &str) -> bool {
    matches!(
        name,
        "say" | "talk" | "wait" | "attempt" | "travel" | "attack" | "give" | "take"
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
    key: String,
    value: bool,
}
