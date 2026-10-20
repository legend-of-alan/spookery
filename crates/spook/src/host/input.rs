//! A player line: `say` is speech, and a verb is a turn.

use spookery_kernel::api::rule::RuleTable;
use spookery_kernel::api::rules::gate::USE_VERB;
use spookery_kernel::api::Reason;

/// `say` is speech. A verb, or `use`, is a turn. A habit name is `NOT_IN_SPEC`.
pub enum PlayerLine {
    Speech,
    Turn,
    Help,
    Map,
    Look,
}

pub fn parse_player_input(
    line: &str,
    seated: &str,
    rules: &RuleTable,
) -> Result<PlayerLine, Reason> {
    let (verb, rest) = verb_line(line, seated)?;
    if verb.is_empty() {
        return Err(Reason::Schema);
    }
    if verb == "say" {
        return Ok(PlayerLine::Speech);
    }
    if rest.is_empty() && matches!(verb, "help" | "map" | "look") {
        return Ok(match verb {
            "help" => PlayerLine::Help,
            "map" => PlayerLine::Map,
            _ => PlayerLine::Look,
        });
    }
    if rules.is_verb(verb) || verb == USE_VERB {
        Ok(PlayerLine::Turn)
    } else {
        Err(Reason::NotInSpec)
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

/// Verb, or verb plus one token, with a single space.
pub fn canonical_line(verb: &str, token: &str) -> String {
    if token.is_empty() {
        verb.to_string()
    } else {
        format!("{verb} {token}")
    }
}
