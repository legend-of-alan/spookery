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

/// Closes the first connection and dials again after the PC choice, since the choice can
/// outlast the first-message deadline.
pub async fn run(addr: SocketAddr, raw: bool) -> Result<(), Error> {
    let mut printer = Printer {
        text: text::Renderer::new(),
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
    drop(seat);
    print_text(&list_pcs(&open, &taken, &pc_names));
    let Some(chosen) = input.recv().await else {
        return Ok(());
    };
    let actor = parse_pc_choice(&chosen)?;
    let mut seat = Connection::open(addr, "player").await?.0;
    let (mut here, mut token) =
        take_claim(&mut seat, json!({ "actor": actor }), &mut printer).await?;
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);
    loop {
        tokio::select! {
            message = seat.recv() => {
                let message = match message {
                    Some(Ok(message)) => message,
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
            line = input.recv() => {
                let Some(line) = line else {
                    return seat.release().await;
                };
                if line.trim() == "release" {
                    return seat.release().await;
                }
                let Some((command, args)) = parse_command(&line, &here) else {
                    continue;
                };
                if !commands.iter().any(|known| known == command) {
                    let refusal = Refusal::from(Reason::NotInSpec);
                    eprintln!("{}", text::render_refusal(&refusal, raw));
                    continue;
                }
                match seat.send(command, args.clone()).await {
                    Ok(_) => {}
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
    let mut seat = match Connection::open(addr, "player").await {
        Ok((seat, _)) => seat,
        Err(error) => {
            tracing::debug!(%error, "could not dial the host again");
            eprintln!("host closed");
            return Ok(None);
        }
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
    text: text::Renderer,
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
    let name = |id: &String| pc_names.get(id).cloned().unwrap_or_default();
    let mut pcs: Vec<String> = open.iter().map(name).collect();
    pcs.extend(taken.iter().map(|id| format!("{} (in use)", name(id))));
    pcs.sort();
    let mut text = format!("Choose a character:\n{}", pcs.join("\n"));
    if open.is_empty() {
        text.push_str("\nNone is free.");
    }
    text
}

/// The PC a picker line names, by id, `player:<id>` or name.
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
/// goes to the table as typed, the first word as the actor in `to` and the rest in `text`, and the
/// table finds who it names. Any other first word is a move with the rest of the line as
/// its target. An empty line names none.
fn parse_command<'l>(line: &'l str, here: &protocol::View) -> Option<(&'l str, Value)> {
    let (command, rest) = line.trim().split_once(' ').unwrap_or((line.trim(), ""));
    if command.is_empty() {
        return None;
    }
    if command == "say" || command == "tell" {
        let (head, text) = rest.split_once(' ').unwrap_or((rest, ""));
        let args = if head == "place" && !text.is_empty() {
            json!({ "to": { "place": here.place }, "text": text })
        } else if !text.is_empty() {
            json!({ "to": { "actor": head }, "text": text })
        } else {
            json!({ "to": "table", "text": rest })
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
    use super::parse_pc_choice;

    #[test]
    fn a_bare_id_and_a_player_prefix_are_one_id() {
        let bare = parse_pc_choice("rowan").ok();
        assert_eq!(bare.as_deref(), Some("rowan"));
        assert_eq!(parse_pc_choice("player:rowan").ok(), bare);
        assert_eq!(parse_pc_choice("  player:rowan").ok(), bare);
        assert!(parse_pc_choice("player:").is_err());
        assert!(parse_pc_choice("").is_err());
    }
}
