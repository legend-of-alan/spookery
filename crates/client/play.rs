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
                let (command, args) = match parse_typed(&typed, &here) {
                    Ok(Some(parsed)) => parsed,
                    Ok(None) => continue,
                    Err(refusal) => {
                        eprintln!("{}", text::render_refusal(&refusal, raw));
                        continue;
                    }
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
    let pcs: Vec<&String> = open.iter().chain(taken).collect();
    let named: Vec<(&str, &str)> = (pcs.iter())
        .map(|id| (id.as_str(), pc_names.get(*id).unwrap_or(id).as_str()))
        .collect();
    let mut lines: Vec<String> = (protocol::add_ids_to_shared_names(&named).into_iter())
        .zip(&pcs)
        .map(|(name, id)| match taken.contains(id) {
            true => format!("{name} (in use)"),
            false => name,
        })
        .collect();
    lines.sort();
    let mut text = format!("Choose a character:\n{}", lines.join("\n"));
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

/// The command a typed line names, and its arguments. `say place <text>` speaks to the place
/// `here` shows, and `say <actor> <text>` to an actor `here` sees, named by id or by the name
/// `here` gives them. Any other first word is a move with the rest of the line as its target.
/// An empty line names none, and a name two actors here share is refused `AMBIGUOUS_NAME`.
fn parse_typed<'l>(
    line: &'l str,
    here: &protocol::View,
) -> Result<Option<(&'l str, Value)>, Refusal> {
    let (command, rest) = line.trim().split_once(' ').unwrap_or((line.trim(), ""));
    if command.is_empty() {
        return Ok(None);
    }
    if command == "say" {
        let (head, text) = rest.split_once(' ').unwrap_or((rest, ""));
        let args = if head == "place" && !text.is_empty() {
            json!({ "address": "place", "place": here.place, "text": text })
        } else if let Some((actor, text)) = split_addressee(rest, here)? {
            json!({ "address": "actor", "actor": actor, "text": text })
        } else {
            json!({ "address": "table", "text": rest })
        };
        return Ok(Some((command, args)));
    }
    if rest.is_empty() {
        return Ok(Some((command, json!({}))));
    }
    Ok(Some((command, json!({ "target": rest }))))
}

/// Split `rest` into the actor `here` sees whose id or name starts it, the longest name
/// first, and the text after it. A name two actors share is refused `AMBIGUOUS_NAME`.
fn split_addressee<'l>(
    rest: &'l str,
    here: &'l protocol::View,
) -> Result<Option<(&'l str, &'l str)>, Refusal> {
    let names = (here.visible_actors.iter().enumerate())
        .map(|(at, id)| (id, here.actor_names.get(at).unwrap_or(id)));
    let names: Vec<(&String, &String)> = names.collect();
    for (at, _) in rest.rmatch_indices(' ') {
        let (head, text) = (&rest[..at], rest[at..].trim());
        let named = |name: &String| protocol::is_named(head, name);
        let mut ids = (names.iter())
            .filter(|(id, name)| named(id) || named(name))
            .map(|(id, _)| id.as_str());
        let Some(id) = ids.next() else {
            continue;
        };
        if ids.next().is_some() {
            return Err(Refusal {
                code: protocol::Code::Known(Reason::AmbiguousName),
                detail: Some(head.trim().to_string()),
            });
        }
        if !text.is_empty() {
            return Ok(Some((id, text)));
        }
    }
    Ok(None)
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
        assert_eq!(
            parse_pc_choice(" Old Rowan ").ok().as_deref(),
            Some("Old Rowan")
        );
        assert!(parse_pc_choice("player:").is_err());
        assert!(parse_pc_choice("").is_err());
    }

    #[test]
    fn a_typed_line_is_a_command_and_its_args() {
        let view: View = serde_json::from_value(json!({
            "viewer": "pc_rowan", "place": "docks_gate", "visible_actors": ["pc_rowan", "mira"],
            "actor_names": ["Rowan", "Mira Vell"],
            "known_facts": [], "notes": [], "unknown_mentions": [], "items": [],
            "habits": []
        }))
        .unwrap();
        let here = &view;
        assert_eq!(parse_typed("wait", here), Ok(Some(("wait", json!({})))));
        assert_eq!(
            parse_typed("travel docks_street", here),
            Ok(Some(("travel", json!({ "target": "docks_street" }))))
        );
        assert_eq!(
            parse_typed("say hi there", here),
            Ok(Some((
                "say",
                json!({ "address": "table", "text": "hi there" })
            )))
        );
        assert_eq!(
            parse_typed("say place hi", here),
            Ok(Some((
                "say",
                json!({ "address": "place", "place": "docks_gate", "text": "hi" })
            )))
        );
        assert_eq!(
            parse_typed("say mira hi", here),
            Ok(Some((
                "say",
                json!({ "address": "actor", "actor": "mira", "text": "hi" })
            )))
        );
        assert_eq!(
            parse_typed("say Mira Vell hi", here),
            Ok(Some((
                "say",
                json!({ "address": "actor", "actor": "mira", "text": "hi" })
            )))
        );
        assert_eq!(parse_typed("  ", here), Ok(None));
        let twins: View = serde_json::from_value(json!({
            "viewer": "pc_rowan", "place": "docks_gate", "visible_actors": ["mira", "mira_twin"],
            "actor_names": ["Mira", "Mira"], "known_facts": [], "notes": [],
            "unknown_mentions": [], "items": [], "habits": []
        }))
        .unwrap();
        let refused = parse_typed("say mira hi", &twins).unwrap_err();
        assert_eq!(refused.detail.as_deref(), Some("mira"));
    }
}
