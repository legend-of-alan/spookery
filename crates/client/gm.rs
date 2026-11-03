//! The human GM: typed lines go out as words, and each message comes back as text.

use std::io::{self, Write};
use std::net::SocketAddr;

use serde_json::{json, Value};

use crate::protocol::{Reason, Refusal, ServerBody, ServerMessage};
use crate::text;
use crate::ws::Connection;
use crate::Error;

const HELP: &str = "\
Commands:
  status
      Who is seated, and the queue.
  perceive <actor>
      What that actor sees. A bare actor id.
  say <text>
      Speak to the whole table.
  queue <actor> <move> [target]
      Queue a move. A target is a place id or an actor id.
  commit
      Commit the queue as one round.
  <word> {...}
      Send a word with these JSON arguments, such as
      say {\"address\": \"place\", \"place\": \"docks_shop\", \"text\": \"...\"}.
  release
      Leave the seat.
  help
      Show these commands.
A word the table does not list, or arguments it cannot read, is not sent.";

/// Sit the GM seat on `/gm` at `addr` until the seat is released or the host closes it.
pub async fn run(addr: SocketAddr, raw: bool) -> Result<(), Error> {
    let (mut seat, hello) = match Connection::open(addr, "gm").await {
        Err(Error::Refused(refusal)) => {
            return Err(Error::Message(text::render_refusal(&refusal, raw)))
        }
        opened => opened?,
    };
    if raw {
        emit(&hello.to_text());
    }
    let words = match hello.body {
        ServerBody::Hello { words, .. } => words,
        _ => Vec::new(),
    };
    emit(HELP);
    let mut input = crate::stdin_lines();
    // Made once, so every Ctrl+C is seen, even one that lands mid-print.
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);
    loop {
        tokio::select! {
            message = seat.recv() => match message {
                Some(Ok(message)) => {
                    if show(&message, raw) {
                        return Ok(());
                    }
                }
                Some(Err(err @ Error::Unreadable(_))) => report(&err.to_string()),
                Some(Err(err)) => return Err(err),
                None => return Ok(()),
            },
            _ = &mut ctrl_c => return seat.release().await,
            typed = input.recv() => {
                let Some(typed) = typed else {
                    return seat.release().await;
                };
                match read_typed(&typed) {
                    None => {}
                    Some(("help", _)) => emit(HELP),
                    Some((word, _)) if !words.iter().any(|known| known == word) => {
                        report(&text::render_refusal(&Refusal::from(Reason::NotInSpec), raw));
                    }
                    Some((word, Some(args))) => {
                        seat.send(word, args).await?;
                    }
                    // A listed word that cannot read the line: its arguments are wrong.
                    Some((_, None)) => {
                        report(&text::render_refusal(&Refusal::from(Reason::Schema), raw));
                    }
                }
            }
        }
    }
}

/// Print one message: its JSON in raw mode, its text otherwise. A refusal also goes to
/// stderr. True once the seat is released.
fn show(message: &ServerMessage, raw: bool) -> bool {
    if raw {
        emit(&message.to_text());
    }
    match &message.body {
        ServerBody::Refused(refusal) => report(&text::render_refusal(refusal, raw)),
        ServerBody::Released => return true,
        body if !raw => {
            let shown = text::render_gm(body);
            if !shown.is_empty() {
                emit(&shown);
            }
        }
        _ => {}
    }
    false
}

fn emit(line: &str) {
    println!("{line}");
    let _ = io::stdout().flush();
}

fn report(line: &str) {
    eprintln!("{line}");
    let _ = io::stderr().flush();
}

/// The word a typed line names and its arguments, or `None` for an empty line. A JSON object
/// after the word is sent as it is. Otherwise `say`, `perceive`, `queue` and `commit` read the
/// rest of the line, and any other word takes nothing after it. No arguments means the word
/// cannot read that line.
fn read_typed(line: &str) -> Option<(&str, Option<Value>)> {
    let line = line.trim();
    let (word, rest) = line
        .split_once(' ')
        .map_or((line, ""), |(word, rest)| (word, rest.trim()));
    if word.is_empty() {
        return None;
    }
    if rest.starts_with('{') {
        let args = serde_json::from_str(rest).ok().filter(Value::is_object);
        return Some((word, args));
    }
    let parts: Vec<&str> = rest.split_whitespace().collect();
    let args = match (word, parts.as_slice()) {
        ("say", [_, ..]) => Some(json!({ "address": "table", "text": rest })),
        ("perceive", [actor]) => Some(json!({ "as": actor })),
        ("queue", [actor, move_name]) => Some(json!({ "actor_id": actor, "move": move_name })),
        ("queue", [actor, move_name, target]) => {
            Some(json!({ "actor_id": actor, "move": move_name, "target": target }))
        }
        ("commit", []) => Some(json!({ "include_round": true })),
        ("say" | "perceive" | "queue" | "commit", _) => None,
        (_, []) => Some(json!({})),
        _ => None,
    };
    Some((word, args))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::read_typed;

    #[test]
    fn a_typed_line_is_a_word_and_its_args() {
        assert_eq!(read_typed("status"), Some(("status", Some(json!({})))));
        assert_eq!(
            read_typed("commit"),
            Some(("commit", Some(json!({ "include_round": true }))))
        );
        assert_eq!(
            read_typed("perceive pc_rowan"),
            Some(("perceive", Some(json!({ "as": "pc_rowan" }))))
        );
        assert_eq!(
            read_typed("say good evening"),
            Some((
                "say",
                Some(json!({ "address": "table", "text": "good evening" }))
            ))
        );
        assert_eq!(
            read_typed("queue mira restock_lamp_oil"),
            Some((
                "queue",
                Some(json!({ "actor_id": "mira", "move": "restock_lamp_oil" }))
            ))
        );
        assert_eq!(
            read_typed("queue pc_rowan travel docks_street"),
            Some((
                "queue",
                Some(json!({ "actor_id": "pc_rowan", "move": "travel", "target": "docks_street" }))
            ))
        );
        assert_eq!(
            read_typed(r#"say {"address": "actor", "actor": "mira", "text": "come in"}"#),
            Some((
                "say",
                Some(json!({ "address": "actor", "actor": "mira", "text": "come in" }))
            ))
        );
    }

    #[test]
    fn a_line_a_word_cannot_read_has_no_args() {
        assert_eq!(read_typed("   "), None);
        assert_eq!(read_typed("status now"), Some(("status", None)));
        assert_eq!(read_typed("perceive"), Some(("perceive", None)));
        assert_eq!(read_typed("perceive a b"), Some(("perceive", None)));
        assert_eq!(read_typed("say"), Some(("say", None)));
        assert_eq!(read_typed("queue mira"), Some(("queue", None)));
        assert_eq!(read_typed("queue a b c d"), Some(("queue", None)));
        assert_eq!(read_typed("commit now"), Some(("commit", None)));
        assert_eq!(read_typed("say {not json"), Some(("say", None)));
        assert_eq!(
            read_typed("say [1]"),
            Some(("say", Some(json!({ "address": "table", "text": "[1]" }))))
        );
    }
}
