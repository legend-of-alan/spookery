//! The human player: claims a PC, turns typed lines into commands, and prints each message.
//! Quitting releases the PC. A dropped connection dials again and takes it back by token.

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::net::SocketAddr;

use serde_json::{json, Value};

use crate::protocol::{self, Reason, Refusal, ServerBody, ServerMessage};
use crate::text;
use crate::ws::Connection;
use crate::Error;

pub async fn run(addr: SocketAddr, raw: bool) -> Result<(), Error> {
    let mut printer = Printer {
        text: text::PlayerText::new(),
        raw,
        last_seq: 0,
    };
    let mut input = crate::stdin_lines();
    let (seat, hello) = Connection::open(addr, "player").await?;
    let (commands, open, taken, pc_names) = match hello.body {
        ServerBody::Hello {
            commands,
            open,
            taken,
            pc_names,
            ..
        } => (commands, open, taken, pc_names),
        _ => Default::default(),
    };
    // The choice may take longer than the first-message timeout, so dial again after.
    drop(seat);
    print_text(&list_pcs(&open, &taken, &pc_names));
    let Some(chosen) = input.recv().await else {
        return Ok(());
    };
    let actor = parse_pc_choice(&chosen)?;
    let mut seat = Connection::open(addr, "player").await?.0;
    let (mut here, mut token) =
        take_claim(&mut seat, json!({ "actor": actor }), &mut printer).await?;
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
                        let Some(back) = reconnect(addr, &token, &mut printer).await? else {
                            return Ok(());
                        };
                        (seat, here, token) = back;
                        continue;
                    }
                    Some(Err(err)) => return Err(err),
                };
                if let Some(shown) = printer.show(&message) {
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
                if typed.trim() == "release" {
                    return seat.release().await;
                }
                let Some((command, args)) = parse_typed(&typed, &here) else {
                    continue;
                };
                // A command the hello did not list points at help, and is not sent.
                if !commands.iter().any(|known| known == command) {
                    let refusal = Refusal::from(Reason::NotInSpec);
                    eprintln!("{}", text::render_refusal(&refusal, raw));
                    continue;
                }
                match seat.send(command, args.clone()).await {
                    Ok(_) => {}
                    // The table dropped this connection: take the PC back, then send again.
                    Err(Error::Io(_)) => {
                        let Some(back) = reconnect(addr, &token, &mut printer).await? else {
                            return Ok(());
                        };
                        (seat, here, token) = back;
                        seat.send(command, args).await?;
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
    printer: &mut Printer,
) -> Result<Option<(Connection, protocol::View, String)>, Error> {
    let Ok((mut seat, _)) = Connection::open(addr, "player").await else {
        eprintln!("host closed");
        return Ok(None);
    };
    let args = json!({ "token": token, "last_seq": printer.last_seq });
    let (here, token) = take_claim(&mut seat, args, printer).await?;
    Ok(Some((seat, here, token)))
}

/// Sends `claim` with `args` and prints its reply, what it missed, and the first view.
/// Returns that view and the claim's token. A refused claim ends play.
async fn take_claim(
    seat: &mut Connection,
    args: Value,
    printer: &mut Printer,
) -> Result<(protocol::View, String), Error> {
    let claim = seat.send("claim", args).await?;
    let mut token = String::new();
    loop {
        let Some(message) = seat.recv().await.transpose()? else {
            return Err(Error::Message("NOT_IN_SPEC".into()));
        };
        if let Some(shown) = printer.show(&message) {
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
                    if let Some(shown) = printer.show(missed) {
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
struct Printer {
    text: text::PlayerText,
    raw: bool,
    last_seq: u64,
}

impl Printer {
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

/// The picker: every PC by the name the hello gives it, the taken ones marked in use.
fn list_pcs(open: &[String], taken: &[String], pc_names: &BTreeMap<String, String>) -> String {
    let name = |id: &String| pc_names.get(id).unwrap_or(id).clone();
    let mut pcs: Vec<String> = open.iter().map(name).collect();
    pcs.extend(taken.iter().map(|id| format!("{} (in use)", name(id))));
    pcs.sort();
    let mut text = format!("Choose a character:\n{}", pcs.join("\n"));
    if open.is_empty() {
        text.push_str("\nNone is free.");
    }
    text
}

/// The PC a picker line names, by id, `player:<id>` or display name.
fn parse_pc_choice(line: &str) -> Result<String, Error> {
    let line = line.trim();
    let choice = line.strip_prefix("player:").unwrap_or(line).trim();
    if choice.is_empty() {
        return Err(Error::Message("SCHEMA".into()));
    }
    Ok(choice.to_string())
}

/// The command a typed line names, and its arguments. `tell` is the word `say`.
/// `say place <text>` speaks to the place `here` shows. Any other say of two or more words
/// goes to the table as typed, the first word in `actor` and the rest in `text`, and the
/// table finds who it names. Any other first word is a move with the rest of the line as
/// its target. An empty line names none.
fn parse_typed<'l>(line: &'l str, here: &protocol::View) -> Option<(&'l str, Value)> {
    let (command, rest) = line.trim().split_once(' ').unwrap_or((line.trim(), ""));
    if command.is_empty() {
        return None;
    }
    if command == "say" || command == "tell" {
        let (head, text) = rest.split_once(' ').unwrap_or((rest, ""));
        let args = if head == "place" && !text.is_empty() {
            json!({ "address": "place", "place": here.place, "text": text })
        } else if !text.is_empty() {
            json!({ "address": "actor", "actor": head, "text": text })
        } else {
            json!({ "address": "table", "text": rest })
        };
        return Some(("say", args));
    }
    if rest.is_empty() {
        return Some((command, json!({})));
    }
    Some((command, json!({ "target": rest })))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{parse_pc_choice, parse_typed};
    use crate::protocol::View;

    #[test]
    fn a_bare_id_and_a_player_prefix_are_one_id() {
        let bare = parse_pc_choice("pc_rowan").ok();
        assert_eq!(bare.as_deref(), Some("pc_rowan"));
        assert_eq!(parse_pc_choice("player:pc_rowan").ok(), bare);
        assert_eq!(parse_pc_choice("  player:pc_rowan").ok(), bare);
        assert!(parse_pc_choice("player:").is_err());
        assert!(parse_pc_choice("").is_err());
    }

    #[test]
    fn a_typed_line_is_a_command_and_its_args() {
        let view: View = serde_json::from_value(json!({
            "viewer": "pc_rowan", "place": "docks_gate", "visible_actors": ["pc_rowan", "mira"],
            "actor_names": ["Rowan", "Mira"],
            "known_facts": [], "notes": [], "unknown_mentions": [], "items": [],
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
            parse_typed("say hi", here),
            Some(("say", json!({ "address": "table", "text": "hi" })))
        );
        assert_eq!(
            parse_typed("say place hi", here),
            Some((
                "say",
                json!({ "address": "place", "place": "docks_gate", "text": "hi" })
            ))
        );
        let typed = json!({ "address": "actor", "actor": "Mira", "text": "Vell hi" });
        assert_eq!(
            parse_typed("say Mira Vell hi", here),
            Some(("say", typed.clone()))
        );
        assert_eq!(parse_typed("tell Mira Vell hi", here), Some(("say", typed)));
        assert_eq!(parse_typed("  ", here), None);
    }
}
