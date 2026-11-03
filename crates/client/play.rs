//! The human player: claims a PC, turns typed lines into words, and prints each message.
//! `help` and `map` are play's own, written from the hello and the views it was shown.
//! Quitting releases the PC. A dropped connection dials again and takes it back by token.

use std::io::{self, Write};
use std::net::SocketAddr;

use serde_json::{json, Value};

use crate::protocol::{self, Reason, Refusal, ServerBody, ServerMessage, Speaker};
use crate::text;
use crate::ws::Connection;
use crate::Error;

pub async fn run(pc: Option<&str>, addr: SocketAddr, raw: bool) -> Result<(), Error> {
    let mut screen = Screen {
        text: text::PlayerText::new(),
        raw,
        last_seq: 0,
    };
    let mut input = crate::stdin_lines();
    let (mut seat, hello) = Connection::open(addr, "player").await?;
    let (words, open) = match hello.body {
        ServerBody::Hello { words, open, .. } => (words, open),
        _ => (Vec::new(), Vec::new()),
    };
    let actor = match pc {
        Some(token) => parse_pc_id(token)?,
        None => match open.as_slice() {
            [] => return Err(Error::Message("No free characters".into())),
            [only] => only.clone(),
            many => {
                // The choice may take longer than the first-message timeout, so dial again after.
                drop(seat);
                println!("Choose a character:\n{}", many.join("\n"));
                let _ = io::stdout().flush();
                let Some(chosen) = input.recv().await else {
                    return Ok(());
                };
                seat = Connection::open(addr, "player").await?.0;
                parse_pc_id(&chosen)?
            }
        },
    };
    let (mut here, mut token) =
        take_claim(&mut seat, json!({ "actor": actor }), &mut screen).await?;
    // Made once, so every Ctrl+C is seen, even one that lands mid-print.
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);
    loop {
        tokio::select! {
            message = seat.recv() => {
                let message = match message {
                    Some(Ok(message)) => message,
                    // The table dropped this connection, or `spook run` stopped.
                    Some(Err(Error::Io(_))) | None => {
                        let Some(back) = reconnect(addr, &token, &mut screen).await? else {
                            return Ok(());
                        };
                        (seat, here, token) = back;
                        continue;
                    }
                    Some(Err(err)) => return Err(err),
                };
                if let Some(shown) = screen.show(&message) {
                    eprintln!("{shown}");
                    let _ = io::stderr().flush();
                }
                if let ServerBody::View(view) = message.body {
                    here = *view;
                }
            }
            _ = &mut ctrl_c => return seat.release().await,
            typed = input.recv() => {
                let Some(typed) = typed else {
                    return seat.release().await;
                };
                match typed.trim() {
                    "release" => return seat.release().await,
                    "help" => {
                        print_text(&text::render_help(&words, &here));
                        continue;
                    }
                    "map" => {
                        print_text(&screen.text.render_map());
                        continue;
                    }
                    _ => {}
                }
                let Some((word, args)) = parse_typed(&typed, &here) else {
                    continue;
                };
                // A word the hello did not list points at help, and is not sent.
                if !words.iter().any(|known| known == word) {
                    let refusal = Refusal::from(Reason::NotInSpec);
                    eprintln!("{}", text::render_refusal(&refusal, raw));
                    continue;
                }
                match seat.send(word, args.clone()).await {
                    Ok(_) => {}
                    // The table dropped this connection: take the PC back, then send again.
                    Err(Error::Io(_)) => {
                        let Some(back) = reconnect(addr, &token, &mut screen).await? else {
                            return Ok(());
                        };
                        (seat, here, token) = back;
                        seat.send(word, args).await?;
                    }
                    Err(err) => return Err(err),
                }
            }
        }
    }
}

/// Dials again and takes the PC back by its token, after the last `seq` read. `None` when
/// `spook run` is gone.
async fn reconnect(
    addr: SocketAddr,
    token: &str,
    screen: &mut Screen,
) -> Result<Option<(Connection, protocol::View, String)>, Error> {
    let Ok((mut seat, _)) = Connection::open(addr, "player").await else {
        eprintln!("host closed");
        return Ok(None);
    };
    let args = json!({ "token": token, "after": screen.last_seq });
    let (here, token) = take_claim(&mut seat, args, screen).await?;
    Ok(Some((seat, here, token)))
}

/// Sends `claim` with `args` and prints its reply, what it missed, and the first view.
/// Returns that view and the claim's token. A refused claim ends play.
async fn take_claim(
    seat: &mut Connection,
    args: Value,
    screen: &mut Screen,
) -> Result<(protocol::View, String), Error> {
    let claim = seat.send("claim", args).await?;
    let mut token = String::new();
    loop {
        let Some(message) = seat.recv().await.transpose()? else {
            return Err(Error::Message("NOT_IN_SPEC".into()));
        };
        if let Some(shown) = screen.show(&message) {
            if message.reply_to.as_deref() == Some(claim.as_str()) {
                return Err(Error::Message(shown));
            }
            eprintln!("{shown}");
        }
        match message.body {
            ServerBody::Claimed {
                token: held,
                missed,
                ..
            } => {
                token = held;
                for missed in &missed {
                    if let Some(shown) = screen.show(missed) {
                        eprintln!("{shown}");
                    }
                }
            }
            ServerBody::View(view) => return Ok((*view, token)),
            _ => {}
        }
    }
}

/// What play prints, and the last `seq` it read, so a reconnect asks for what came after.
struct Screen {
    text: text::PlayerText,
    raw: bool,
    last_seq: u64,
}

impl Screen {
    /// Print one message. A refusal is returned instead, as the line to show on stderr: its
    /// code in raw mode, its sentence in text mode.
    fn show(&mut self, message: &ServerMessage) -> Option<String> {
        self.last_seq = self.last_seq.max(message.seq.unwrap_or(0));
        if let ServerBody::Refused(refusal) = &message.body {
            return Some(text::render_refusal(refusal, self.raw));
        }
        let shown = if self.raw {
            message.to_text()
        } else {
            self.text.render(&message.body)
        };
        if !shown.is_empty() {
            print_text(&shown);
        }
        None
    }
}

fn print_text(text: &str) {
    println!("{text}");
    let _ = io::stdout().flush();
}

fn parse_pc_id(token: &str) -> Result<String, Error> {
    let token = token.trim();
    let id = token.strip_prefix("player:").unwrap_or(token);
    match format!("player:{id}").parse() {
        Ok(Speaker::Player(id)) => Ok(id),
        _ => Err(Error::Message("SCHEMA".into())),
    }
}

/// The word a typed line names, and its arguments. `say place <text>` speaks to the place
/// `here` shows, and `say <actor> <text>` to an actor `here` sees. Any other first word is a
/// move with the rest of the line as its target. An empty line names none.
fn parse_typed<'l>(line: &'l str, here: &protocol::View) -> Option<(&'l str, Value)> {
    let (word, rest) = line.trim().split_once(' ').unwrap_or((line.trim(), ""));
    if word.is_empty() {
        return None;
    }
    if word == "say" {
        let (head, text) = rest.split_once(' ').unwrap_or((rest, ""));
        let args = if head == "place" && !text.is_empty() {
            json!({ "address": "place", "place": here.place, "text": text })
        } else if !text.is_empty() && here.visible_actors.iter().any(|id| id == head) {
            json!({ "address": "actor", "actor": head, "text": text })
        } else {
            json!({ "address": "table", "text": rest })
        };
        return Some((word, args));
    }
    if rest.is_empty() {
        return Some((word, json!({})));
    }
    Some((word, json!({ "target": rest })))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{parse_pc_id, parse_typed};
    use crate::protocol::View;

    #[test]
    fn a_bare_id_and_a_player_prefix_are_one_id() {
        let bare = parse_pc_id("pc_rowan").ok();
        assert_eq!(bare.as_deref(), Some("pc_rowan"));
        assert_eq!(parse_pc_id("player:pc_rowan").ok(), bare);
        assert_eq!(parse_pc_id("  player:pc_rowan").ok(), bare);
        assert!(parse_pc_id("player:").is_err());
        assert!(parse_pc_id("").is_err());
    }

    #[test]
    fn a_typed_line_is_a_word_and_its_args() {
        let view: View = serde_json::from_value(json!({
            "viewer": "pc_rowan", "place": "docks_gate", "visible_actors": ["pc_rowan", "mira"],
            "known_facts": [], "notes": [], "unknown_mentions": [], "clocks": [], "items": [],
            "habits": []
        }))
        .unwrap();
        let here = &view;
        assert_eq!(parse_typed("wait", here), Some(("wait", json!({}))));
        assert_eq!(
            parse_typed("travel docks_street", here),
            Some(("travel", json!({ "target": "docks_street" })))
        );
        assert_eq!(
            parse_typed("say hi there", here),
            Some(("say", json!({ "address": "table", "text": "hi there" })))
        );
        assert_eq!(
            parse_typed("say place hi", here),
            Some((
                "say",
                json!({ "address": "place", "place": "docks_gate", "text": "hi" })
            ))
        );
        assert_eq!(
            parse_typed("say mira hi", here),
            Some((
                "say",
                json!({ "address": "actor", "actor": "mira", "text": "hi" })
            ))
        );
        assert_eq!(parse_typed("  ", here), None);
    }
}
