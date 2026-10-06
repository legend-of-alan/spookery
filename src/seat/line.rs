use std::collections::BTreeMap;

use spookery_kernel::moves::{Audience, MoveDef};
use spookery_kernel::Reason;

/// `say` and `talk` are speech. A player-audience row is a turn.
/// A habit or system name is `NOT_IN_SPEC` and is not queued.
pub enum PlayerLine {
    Speech,
    Turn,
}

pub fn classify_player_line(
    line: &str,
    seated: &str,
    moves: &BTreeMap<String, MoveDef>,
) -> Result<PlayerLine, Reason> {
    let (verb, _) = verb_line(line, seated)?;
    if verb.is_empty() {
        return Err(Reason::Schema);
    }
    if verb == "say" || verb == "talk" {
        return Ok(PlayerLine::Speech);
    }
    match moves.get(verb).and_then(|def| def.row.as_ref()) {
        Some(row) if row.audience == Some(Audience::Player) => Ok(PlayerLine::Turn),
        Some(_) => Err(Reason::NotInSpec),
        None => Err(Reason::NotInSpec),
    }
}

pub fn verb_line<'a>(line: &'a str, seated: &str) -> Result<(&'a str, &'a str), Reason> {
    if line.is_empty() {
        return Err(Reason::Schema);
    }
    let (first, rest) = split_token(line);
    if let Some(id) = first.strip_prefix("player:") {
        if id.is_empty() || id != seated || rest.is_empty() {
            return Err(Reason::Schema);
        }
        return Ok(split_token(rest));
    }
    Ok((first, rest))
}

pub fn split_token(line: &str) -> (&str, &str) {
    match line.split_once(' ') {
        Some((token, rest)) => (token, rest),
        None => (line, ""),
    }
}

/// Row name, or row name plus one token, with a single space.
pub fn canonical_line(verb: &str, token: &str) -> String {
    if token.is_empty() {
        verb.to_string()
    } else {
        format!("{verb} {token}")
    }
}
