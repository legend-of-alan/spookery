use std::collections::{btree_map::Entry, BTreeMap};

use serde::Deserialize;

use crate::diagnostic::{locate_row_field, parse_file, schema, tag_diagnostic, Diagnostic};
use crate::meter::{MeterWrite, WorkWrite};

const MOVES_FILE: &str = "rules/moves.yaml";

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
    Memory {
        fact_ids: Vec<String>,
        tags: Vec<String>,
        text: Option<String>,
    },
}

pub fn load_moves(text: &str) -> Result<BTreeMap<String, MoveDef>, Vec<Diagnostic>> {
    if let Some(diag) = tag_diagnostic(text, crate::format::MOVES, MOVES_FILE, 0) {
        return Err(vec![diag]);
    }
    let file: MovesFile = parse_file(MOVES_FILE, text, 0)?;
    let mut diags = Vec::new();
    if file.schema != crate::format::MOVES {
        diags.push(
            schema(
                MOVES_FILE,
                &format!("schema is '{}', which this file does not use", file.schema),
                "write 'schema: moves' with no version suffix",
            )
            .with_path("schema"),
        );
    }
    let mut moves = BTreeMap::new();
    for (index, entry) in file.moves.into_iter().enumerate() {
        match move_def(text, index, entry) {
            Ok((name, def)) => match moves.entry(name.clone()) {
                Entry::Occupied(_) => diags.push(row_problem(
                    text,
                    index,
                    "name:",
                    &format!("moves[{index}].name"),
                    &format!("move '{name}' is listed twice"),
                    "keep one row",
                )),
                Entry::Vacant(slot) => {
                    slot.insert(def);
                }
            },
            Err(more) => diags.extend(more),
        }
    }
    if diags.is_empty() {
        Ok(moves)
    } else {
        Err(diags)
    }
}

fn move_def(
    text: &str,
    index: usize,
    mut entry: MoveEntry,
) -> Result<(String, MoveDef), Vec<Diagnostic>> {
    let mut diags = Vec::new();
    let name = entry.name.clone();
    if name.is_empty() {
        diags.push(row_problem(
            text,
            index,
            "name:",
            &format!("moves[{index}].name"),
            "a move name is empty",
            "set name to a short id",
        ));
    }
    if entry.item.as_ref().is_some_and(|id| id.is_empty()) {
        diags.push(row_problem(
            text,
            index,
            "item:",
            &format!("moves[{index}].item"),
            "item is empty",
            "name an item the actor already holds, or remove item",
        ));
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
        return if diags.is_empty() {
            Ok((
                entry.name,
                MoveDef {
                    check: entry.check,
                    row: None,
                },
            ))
        } else {
            Err(diags)
        };
    }
    if forbids_row(&name) {
        diags.push(row_problem(
            text,
            index,
            "name:",
            &format!("moves[{index}].name"),
            "row 'say' is speech, not a move",
            "remove the row",
        ));
    }
    let audience = match entry.audience.as_deref() {
        Some("player") => Some(Audience::Player),
        Some("habit") => Some(Audience::Habit),
        Some(other) => {
            diags.push(row_problem(
                text,
                index,
                "audience:",
                &format!("moves[{index}].audience"),
                &format!("row '{name}' has audience '{other}', which is not player or habit"),
                "use 'audience: player' or 'audience: habit'",
            ));
            None
        }
        None => {
            diags.push(row_problem(
                text,
                index,
                "name:",
                &format!("moves[{index}].audience"),
                &format!("row '{name}' has no audience"),
                "add 'audience: player' or 'audience: habit'",
            ));
            None
        }
    };
    // `effects: []` with neither outcome list is a yield. An empty success or failure list is not.
    let yield_row = entry.effects.as_ref().is_some_and(Vec::is_empty)
        && entry.success.is_none()
        && entry.failure.is_none();
    if entry.effects.as_ref().is_some_and(Vec::is_empty) && !yield_row {
        diags.push(row_problem(
            text,
            index,
            "effects:",
            &format!("moves[{index}].effects"),
            &format!("row '{name}' has an empty effects list beside another list"),
            "remove effects, or make this a yield with only 'effects: []'",
        ));
    }
    if entry.success.as_ref().is_some_and(Vec::is_empty) {
        diags.push(row_problem(
            text,
            index,
            "success:",
            &format!("moves[{index}].success"),
            &format!("row '{name}' has an empty success list"),
            "add an effect or remove success",
        ));
    }
    if entry.failure.as_ref().is_some_and(Vec::is_empty) {
        diags.push(row_problem(
            text,
            index,
            "failure:",
            &format!("moves[{index}].failure"),
            &format!("row '{name}' has an empty failure list"),
            "add an effect or remove failure",
        ));
    }
    let token = match entry.token.as_deref() {
        Some("none") => Some(MoveToken::None),
        Some("actor") => Some(MoveToken::Actor),
        Some("place") => Some(MoveToken::Place),
        Some(other) => {
            diags.push(row_problem(
                text,
                index,
                "token:",
                &format!("moves[{index}].token"),
                &format!("row '{name}' has token '{other}'"),
                "set token to none, actor, or place",
            ));
            None
        }
        None => {
            diags.push(row_problem(
                text,
                index,
                "name:",
                &format!("moves[{index}].token"),
                &format!("row '{name}' has no token"),
                "set token to none, actor, or place",
            ));
            None
        }
    };
    let on = match entry.on.as_deref() {
        Some("speaker") => Some(MoveOn::Speaker),
        Some("named") => Some(MoveOn::Named),
        Some(other) => {
            diags.push(row_problem(
                text,
                index,
                "on:",
                &format!("moves[{index}].on"),
                &format!("row '{name}' has on '{other}'"),
                "set on to speaker or named",
            ));
            None
        }
        None => {
            diags.push(row_problem(
                text,
                index,
                "name:",
                &format!("moves[{index}].on"),
                &format!("row '{name}' has no on"),
                "set on to speaker or named",
            ));
            None
        }
    };
    let (effects, success, failure) = match (token, on) {
        (Some(token), Some(on)) if pair_ok(token, on) => {
            parse_lists(text, index, token, on, &mut entry, &mut diags)
        }
        (Some(token), Some(on)) => {
            diags.push(row_problem(
                text,
                index,
                "token:",
                &format!("moves[{index}].token"),
                &format!(
                    "row '{name}' pairs token {} with on {}",
                    token_word(token),
                    on_word(on)
                ),
                "use none with speaker, actor with named, or place with speaker",
            ));
            (Vec::new(), Vec::new(), Vec::new())
        }
        _ => (Vec::new(), Vec::new(), Vec::new()),
    };
    if !yield_row
        && effects.is_empty()
        && success.is_empty()
        && failure.is_empty()
        && diags.is_empty()
    {
        diags.push(row_problem(
            text,
            index,
            "name:",
            &format!("moves[{index}].effects"),
            &format!("row '{name}' has no effects"),
            "add an effect, or use 'effects: []' alone as a yield",
        ));
    }
    if !diags.is_empty() {
        return Err(diags);
    }
    let (Some(audience), Some(token), Some(on)) = (audience, token, on) else {
        return Err(vec![schema(
            MOVES_FILE,
            &format!("row '{name}' could not be read"),
            "fix the row and try again",
        )]);
    };
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

fn parse_lists(
    text: &str,
    index: usize,
    token: MoveToken,
    on: MoveOn,
    entry: &mut MoveEntry,
    diags: &mut Vec<Diagnostic>,
) -> (Vec<RowEffect>, Vec<RowEffect>, Vec<RowEffect>) {
    let effects = match entry.effects.take() {
        Some(items) if !items.is_empty() => {
            match templates_for(text, index, "effects", token, on, items) {
                Ok(parsed) => parsed,
                Err(more) => {
                    diags.extend(more);
                    Vec::new()
                }
            }
        }
        _ => Vec::new(),
    };
    let success = match entry.success.take() {
        Some(items) if !items.is_empty() => {
            match outcome_templates(text, index, "success", token, on, items) {
                Ok(parsed) => parsed,
                Err(more) => {
                    diags.extend(more);
                    Vec::new()
                }
            }
        }
        _ => Vec::new(),
    };
    let failure = match entry.failure.take() {
        Some(items) if !items.is_empty() => {
            match outcome_templates(text, index, "failure", token, on, items) {
                Ok(parsed) => parsed,
                Err(more) => {
                    diags.extend(more);
                    Vec::new()
                }
            }
        }
        _ => Vec::new(),
    };
    (effects, success, failure)
}

fn row_problem(
    text: &str,
    index: usize,
    field: &str,
    path: &str,
    problem: &str,
    hint: &str,
) -> Diagnostic {
    let mut diag = schema(MOVES_FILE, problem, hint).with_path(path);
    if let Some((line, column)) = locate_row_field(text, "name:", index, field) {
        diag = diag.at_line(line, Some(column));
    }
    diag
}

fn token_word(token: MoveToken) -> &'static str {
    match token {
        MoveToken::None => "none",
        MoveToken::Actor => "actor",
        MoveToken::Place => "place",
    }
}

fn on_word(on: MoveOn) -> &'static str {
    match on {
        MoveOn::Speaker => "speaker",
        MoveOn::Named => "named",
    }
}

fn templates_for(
    text: &str,
    index: usize,
    list: &str,
    token: MoveToken,
    on: MoveOn,
    effects: Vec<EffectTemplate>,
) -> Result<Vec<RowEffect>, Vec<Diagnostic>> {
    let mut templates = Vec::with_capacity(effects.len());
    let mut diags = Vec::new();
    for (nth, effect) in effects.into_iter().enumerate() {
        let at = At {
            text,
            index,
            path: format!("moves[{index}].{list}[{nth}]"),
        };
        match one_template(&at, token, on, effect) {
            Ok(template) => templates.push(template),
            Err(more) => diags.extend(more),
        }
    }
    if diags.is_empty() {
        Ok(templates)
    } else {
        Err(diags)
    }
}

/// A place token still has neither outcome list.
fn outcome_templates(
    text: &str,
    index: usize,
    list: &str,
    token: MoveToken,
    on: MoveOn,
    effects: Vec<EffectTemplate>,
) -> Result<Vec<RowEffect>, Vec<Diagnostic>> {
    match (token, on) {
        (MoveToken::None, MoveOn::Speaker) | (MoveToken::Actor, MoveOn::Named) => {
            templates_for(text, index, list, token, on, effects)
        }
        _ => Err(vec![row_problem(
            text,
            index,
            &format!("{list}:"),
            &format!("moves[{index}].{list}"),
            &format!(
                "this row cannot use {list} with token {} and on {}",
                token_word(token),
                on_word(on)
            ),
            "use token none with on speaker, or token actor with on named",
        )]),
    }
}

struct At<'a> {
    text: &'a str,
    index: usize,
    path: String,
}

impl At<'_> {
    fn fail(&self, field: &str, problem: &str, hint: &str) -> Vec<Diagnostic> {
        vec![self.problem(field, problem, hint)]
    }

    fn problem(&self, field: &str, problem: &str, hint: &str) -> Diagnostic {
        let mut diag = schema(MOVES_FILE, problem, hint).with_path(self.path.clone());
        if let Some((line, column)) = locate_row_field(self.text, "name:", self.index, field) {
            diag = diag.at_line(line, Some(column));
        }
        diag
    }
}

fn one_template(
    at: &At<'_>,
    token: MoveToken,
    on: MoveOn,
    effect: EffectTemplate,
) -> Result<RowEffect, Vec<Diagnostic>> {
    if !pair_ok(token, on) {
        return Err(at.fail(
            "token:",
            "token and on are not a legal pair",
            "use none with speaker, actor with named, or place with speaker",
        ));
    }
    match effect.class.as_str() {
        "flag" => flag_template(at, token, on, &effect),
        "location" => location_template(at, token, &effect),
        "meter" => meter_template(at, &effect),
        "stock" => stock_template(at, &effect),
        "clock" => clock_template(at, &effect),
        "message" => message_template(at, &effect),
        "memory" => memory_template(at, &effect),
        other => Err(at.fail(
            "class:",
            &format!(
                "effect class '{other}' is not one of flag, location, meter, stock, clock, message, or memory"
            ),
            "use one of those classes",
        )),
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
    at: &At<'_>,
    token: MoveToken,
    on: MoveOn,
    effect: &EffectTemplate,
) -> Result<RowEffect, Vec<Diagnostic>> {
    only(at, "flag", effect, &["key", "value"])?;
    let flag_pair = matches!(
        (token, on),
        (MoveToken::None, MoveOn::Speaker) | (MoveToken::Actor, MoveOn::Named)
    );
    if !flag_pair {
        return Err(at.fail(
            "class:",
            "a flag effect needs token none or actor",
            "change the token, or use another class",
        ));
    }
    let Some(key) = effect.key.clone() else {
        return Err(at.fail("key:", "flag needs a key", "add key"));
    };
    let Some(value) = effect.value else {
        return Err(at.fail(
            "value:",
            "flag needs a value",
            "add 'value: true' or 'value: false'",
        ));
    };
    if key == "busy" {
        return Err(at.fail("key:", "flag 'busy' is not a row effect", "remove it"));
    }
    Ok(RowEffect::Flag { key, value })
}

fn location_template(
    at: &At<'_>,
    token: MoveToken,
    effect: &EffectTemplate,
) -> Result<RowEffect, Vec<Diagnostic>> {
    only(at, "location", effect, &["to"])?;
    match effect.to.as_ref() {
        None if token == MoveToken::Place => Ok(RowEffect::Location),
        Some(ToField::One(word)) if word == "token" => {
            if token != MoveToken::Place {
                return Err(at.fail(
                    "to:",
                    "to: token needs token place",
                    "set token to place, or name a place id",
                ));
            }
            Ok(RowEffect::Location)
        }
        Some(ToField::One(word)) => Ok(RowEffect::LocationTo {
            to: literal_id(at, "to:", Some(word))?,
        }),
        _ => Err(at.fail(
            "to:",
            "location to is not a place id or token",
            "set 'to: token' or a place id",
        )),
    }
}

fn meter_template(at: &At<'_>, effect: &EffectTemplate) -> Result<RowEffect, Vec<Diagnostic>> {
    only(at, "meter", effect, &["key", "write", "delta"])?;
    let key = effect.key.clone().ok_or_else(|| {
        at.fail(
            "key:",
            "meter needs a key",
            "name a meter from rules/meters.yaml",
        )
    })?;
    let write = match (effect.write.as_deref(), effect.delta.as_ref()) {
        (Some("base"), Some(DeltaField::Int(delta))) => MeterWrite::Base(*delta),
        (Some("offset"), Some(DeltaField::Int(delta))) => MeterWrite::Offset(*delta),
        (Some("snap"), None) => MeterWrite::Snap,
        _ => {
            return Err(at.fail(
                "write:",
                "meter write must be base, offset, or snap",
                "set write, and set delta to a number unless write is snap",
            ))
        }
    };
    Ok(RowEffect::Meter(WorkWrite { key, write }))
}

fn stock_template(at: &At<'_>, effect: &EffectTemplate) -> Result<RowEffect, Vec<Diagnostic>> {
    only(at, "stock", effect, &["place", "sku", "delta"])?;
    Ok(RowEffect::Stock {
        place: literal_id(at, "place:", effect.place.as_deref())?,
        sku: literal_id(at, "sku:", effect.sku.as_deref())?,
        delta: required_number(at, effect.delta.as_ref())?,
    })
}

fn clock_template(at: &At<'_>, effect: &EffectTemplate) -> Result<RowEffect, Vec<Diagnostic>> {
    only(at, "clock", effect, &["clock", "delta"])?;
    Ok(RowEffect::Clock {
        clock: literal_id(at, "clock:", effect.clock.as_deref())?,
        delta: required_number(at, effect.delta.as_ref())?,
    })
}

fn message_template(at: &At<'_>, effect: &EffectTemplate) -> Result<RowEffect, Vec<Diagnostic>> {
    only(
        at,
        "message",
        effect,
        &["id", "to", "place", "fact_ids", "text"],
    )?;
    let id = literal_id(at, "id:", effect.id.as_deref())?;
    let to = match effect.to.as_ref() {
        Some(ToField::Many(ids)) if !ids.is_empty() => ids
            .iter()
            .map(|id| literal_id(at, "to:", Some(id.as_str())))
            .collect::<Result<Vec<_>, _>>()?,
        _ => return Err(at.fail("to:", "message needs a to list", "name at least one actor")),
    };
    let place = match effect.place.as_deref() {
        None => None,
        Some(place) => Some(literal_id(at, "place:", Some(place))?),
    };
    let fact_ids = effect
        .fact_ids
        .iter()
        .flatten()
        .map(|id| literal_id(at, "fact_ids:", Some(id.as_str())))
        .collect::<Result<Vec<_>, _>>()?;
    let text = effect
        .text
        .clone()
        .ok_or_else(|| at.fail("text:", "message needs text", "add the sentence"))?;
    Ok(RowEffect::Message {
        id,
        to,
        place,
        fact_ids,
        text,
    })
}

fn memory_template(at: &At<'_>, effect: &EffectTemplate) -> Result<RowEffect, Vec<Diagnostic>> {
    only(at, "memory", effect, &["fact_ids", "tags", "text"])?;
    Ok(RowEffect::Memory {
        fact_ids: effect.fact_ids.clone().ok_or_else(|| {
            at.fail(
                "fact_ids:",
                "memory needs fact_ids",
                "add the list, even if it is empty",
            )
        })?,
        tags: effect.tags.clone().ok_or_else(|| {
            at.fail(
                "tags:",
                "memory needs tags",
                "add the list, even if it is empty",
            )
        })?,
        text: effect.text.clone(),
    })
}

fn only(
    at: &At<'_>,
    class: &str,
    effect: &EffectTemplate,
    allowed: &[&str],
) -> Result<(), Vec<Diagnostic>> {
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
        ("fact_ids", effect.fact_ids.is_some()),
        ("tags", effect.tags.is_some()),
        ("text", effect.text.is_some()),
    ];
    if let Some((name, _)) = present
        .into_iter()
        .find(|(name, set)| *set && !allowed.contains(name))
    {
        return Err(at.fail(
            &format!("{name}:"),
            &format!("field '{name}' does not belong on class '{class}'"),
            &format!("remove '{name}'"),
        ));
    }
    Ok(())
}

fn literal_id(at: &At<'_>, field: &str, word: Option<&str>) -> Result<String, Vec<Diagnostic>> {
    let Some(word) = word else {
        return Err(at.fail(field, &format!("{field} is missing"), "set it to an id"));
    };
    if word.is_empty() || word == "token" || retired_word(word) {
        return Err(at.fail(
            field,
            &format!("'{word}' is not an id"),
            "write the id itself, not a selector word",
        ));
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

fn required_number(at: &At<'_>, delta: Option<&DeltaField>) -> Result<i64, Vec<Diagnostic>> {
    match delta {
        Some(DeltaField::Int(n)) => Ok(*n),
        _ => Err(at.fail("delta:", "delta must be a number", "write an integer")),
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
    fact_ids: Option<Vec<String>>,
    #[serde(default)]
    tags: Option<Vec<String>>,
    #[serde(default)]
    text: Option<String>,
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
    use crate::diagnostic::reason_of;
    use crate::error::Reason;
    use crate::meter::{MeterWrite, WorkWrite};

    fn schema(body: &str) -> Reason {
        let text = format!("schema: moves\nmoves:\n{body}");
        let err = load_moves(&text).expect_err("schema");
        assert!(
            err.iter().all(|diag| {
                !diag.file.is_empty()
                    && !diag.message.is_empty()
                    && diag.hint.is_some()
                    && (diag.line.is_some() || diag.path.is_some())
            }),
            "{err:?}"
        );
        reason_of(err)
    }

    #[test]
    fn an_unknown_tag_and_the_old_versioned_tag_are_schema() {
        let text = |tag: &str| format!("schema: {tag}\nmoves: []\n");
        assert!(load_moves(&text("moves")).is_ok());
        let old = load_moves(&text(concat!("moves/", "v0"))).unwrap_err();
        assert!(old[0].message.contains("no version"), "{old:?}");
        assert_eq!(reason_of(old), Reason::Schema);
        let bogus = load_moves(&text("bogus")).unwrap_err();
        assert!(bogus[0].message.contains("does not use"), "{bogus:?}");
        assert_eq!(reason_of(bogus), Reason::Schema);
    }

    #[test]
    fn loader_messages_name_the_fix() {
        let missing = schema("  - name: grit\n    token: none\n    on: speaker\n    effects: []\n");
        assert_eq!(missing, Reason::Schema);
        let text = "schema: moves\nmoves:\n  - name: grit\n    token: none\n    on: speaker\n    effects: []\n";
        let err = load_moves(text).unwrap_err();
        assert!(
            err.iter().any(|diag| {
                diag.path.as_deref() == Some("moves[0].audience")
                    && diag.message.contains("has no audience")
                    && diag.message.contains("audience: habit")
            }),
            "{err:?}"
        );
        let duplicate = "schema: moves\nmoves:\n  - name: grit\n    audience: player\n    token: none\n    on: speaker\n    effects: []\n  - name: grit\n    audience: player\n    token: none\n    on: speaker\n    effects: []\n";
        let err = load_moves(duplicate).unwrap_err();
        assert!(
            err.iter().any(|diag| diag.message.contains("listed twice")),
            "{err:?}"
        );
        let unknown = "schema: moves\nmoves:\n  - name: grit\n    audiance: player\n";
        let err = load_moves(unknown).unwrap_err();
        assert_eq!(err.len(), 1, "{err:?}");
        assert!(
            err[0].message.contains("unknown field 'audiance'"),
            "{err:?}"
        );
        assert!(err[0].line.is_some(), "{err:?}");
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
    fn a_row_names_seven_classes_literals_and_a_yield() {
        let moves = load_moves(
            r#"schema: moves
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
      - class: memory
        fact_ids: []
        tags: []
      - class: flag
        key: marked
        value: true
  - name: night
    audience: habit
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
            shelf.effects[1],
            RowEffect::Meter(WorkWrite {
                key: "rest".to_string(),
                write: MeterWrite::Base(1),
            })
        );
        let night = moves.get("night").unwrap().row.as_ref().unwrap();
        assert_eq!(night.audience, Some(Audience::Habit));
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
            r#"schema: moves
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
                "  - name: night\n    audience: system\n    token: none\n    on: speaker\n    effects: []\n"
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
                "  - name: mark\n    audience: habit\n    token: none\n    on: speaker\n    effects:\n      - class: relationship\n        to: bram\n        delta: -1\n"
            ),
            Reason::Schema
        );
        assert_eq!(
            schema(
                "  - name: mark\n    audience: habit\n    token: none\n    on: speaker\n    effects:\n      - class: reputation\n        faction: harbor\n        delta: 1\n"
            ),
            Reason::Schema
        );
        assert_eq!(
            schema(
                "  - name: mark\n    audience: habit\n    token: none\n    on: speaker\n    effects:\n      - class: rumor\n        rumor: r_184\n        to: aldric\n        heat: 1\n"
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
