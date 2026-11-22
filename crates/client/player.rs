//! The human player: claims a PC, turns typed lines into commands, and shows each message as
//! lines or in the game window. Quitting releases the PC. A dropped connection opens again and
//! takes it back by token. Opens a fresh connection after the PC choice, so a slow choice never
//! trips the first-message deadline. If the host is gone, returns `ExitReason::HostClosed` so the
//! caller can print `host closed` after the terminal is restored.

use std::net::SocketAddr;

use serde_json::{json, Value};

use crate::protocol::{self, Reason, Refusal, ServerBody, ServerMessage};
use crate::terminal::Terminal;
use crate::text;
use crate::websocket;
use crate::Error;

/// How play exited.
pub enum ExitReason {
    /// The player left, releasing their PC if they held one.
    PlayerLeft,
    /// `spook run` was gone when play opened a connection again.
    HostClosed,
}

/// Connects, claims a PC, and plays until the player leaves or the host closes.
pub async fn play(addr: SocketAddr, terminal: &mut Terminal) -> Result<ExitReason, Error> {
    let (connection, hello) = websocket::Connection::open(addr, "player").await?;
    let commands = match &hello.body {
        ServerBody::Hello { commands, .. } => commands
            .iter()
            .filter(|command| *command != "claim" && *command != "release")
            .cloned()
            .collect(),
        _ => Vec::new(),
    };
    terminal.show(&hello, None)?;
    drop(connection);
    let Some(chosen) = terminal.next_line(None, &commands).await? else {
        return Ok(ExitReason::PlayerLeft);
    };
    let actor = chosen.trim();
    if actor.is_empty() {
        return Err(Error::Message("SCHEMA".into()));
    }
    let mut connection = websocket::Connection::open(addr, "player").await?.0;
    let (mut view, mut token) =
        claim_pc(&mut connection, json!({ "actor": actor }), terminal, None).await?;
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);
    loop {
        tokio::select! {
            message = connection.recv() => {
                let message = match message {
                    Some(Ok(message)) => message,
                    Some(Err(Error::Io(_))) | None => {
                        if !reconnect(addr, &mut connection, &mut view, &mut token, terminal).await?
                        {
                            return Ok(ExitReason::HostClosed);
                        }
                        continue;
                    }
                    Some(Err(err)) => return Err(err),
                };
                if let ServerBody::View(body) = &message.body {
                    view = (**body).clone();
                }
                terminal.show(&message, Some(&view))?;
            }
            _ = &mut ctrl_c => break,
            line = terminal.next_line(Some(&view), &commands) => {
                let Some(line) = line? else {
                    break;
                };
                if line.trim() == "quit" {
                    break;
                }
                let Some((command, args)) = parse_command(&line, &view) else {
                    continue;
                };
                if !commands.iter().any(|known| known == command) {
                    terminal.show(
                        &ServerMessage {
                            id: None,
                            seq: None,
                            reply_to: None,
                            body: ServerBody::Refused(Refusal::from(Reason::NotInSpec)),
                        },
                        Some(&view),
                    )?;
                    continue;
                }
                match connection.send(command, args.clone()).await {
                    Ok(_) => {}
                    Err(Error::Io(_)) => {
                        if !reconnect(addr, &mut connection, &mut view, &mut token, terminal).await?
                        {
                            return Ok(ExitReason::HostClosed);
                        }
                        connection.send(command, args).await?;
                    }
                    Err(err) => return Err(err),
                }
            }
        }
    }
    connection.release().await?;
    Ok(ExitReason::PlayerLeft)
}

/// Opens a connection again and takes the PC back by its token, after the last `seq` shown.
/// Replaces the live connection, view and token only when that succeeds. `false` when
/// `spook run` is gone, leaving those live values untouched.
async fn reconnect(
    addr: SocketAddr,
    connection: &mut websocket::Connection,
    view: &mut protocol::View,
    token: &mut String,
    terminal: &mut Terminal,
) -> Result<bool, Error> {
    let mut opened = match websocket::Connection::open(addr, "player").await {
        Ok((connection, _)) => connection,
        Err(error) => {
            tracing::debug!(%error, "could not connect to the host again");
            return Ok(false);
        }
    };
    let args = json!({ "token": token.as_str(), "last_seq": terminal.last_seq() });
    let (new_view, new_token) = claim_pc(&mut opened, args, terminal, Some(view.clone())).await?;
    *connection = opened;
    *view = new_view;
    *token = new_token;
    Ok(true)
}

/// Sends `claim` with `args` and shows its reply, what it missed, and the first view.
/// Returns that view and the token. If the table refuses, play ends.
async fn claim_pc(
    connection: &mut websocket::Connection,
    args: Value,
    terminal: &mut Terminal,
    mut view: Option<protocol::View>,
) -> Result<(protocol::View, String), Error> {
    let request_id = connection.send("claim", args).await?;
    let mut token = String::new();
    loop {
        let Some(server_message) = connection.recv().await.transpose()? else {
            return Err(Error::Message("NOT_IN_SPEC".into()));
        };
        if let ServerBody::Refused(refusal) = &server_message.body {
            if server_message.reply_to.as_deref() == Some(request_id.as_str()) {
                return Err(Error::Message(text::render_refusal(
                    refusal,
                    terminal.is_raw(),
                )));
            }
        }
        let missed_messages = match &server_message.body {
            ServerBody::Claimed {
                missed: missed_messages,
                ..
            } => missed_messages.as_slice(),
            _ => &[],
        };
        let messages = std::iter::once(&server_message).chain(missed_messages);
        for message in messages {
            if let ServerBody::View(body) = &message.body {
                view = Some((**body).clone());
            }
            terminal.show(message, view.as_ref())?;
        }
        match server_message.body {
            ServerBody::Claimed { token: held, .. } => token = held,
            ServerBody::View(body) => return Ok((*body, token)),
            _ => {}
        }
    }
}

/// The command a typed line names, and its arguments. `tell` is the word `say`.
/// `say place <text>` speaks to the place `view` shows. Any other say of two or more words
/// goes to the table as typed, the first word as the actor in `to` and the rest in `text`, and the
/// table finds who it names. Any other first word is a move with the rest of the line as
/// its target. An empty line names none.
fn parse_command<'l>(line: &'l str, view: &protocol::View) -> Option<(&'l str, Value)> {
    let (command, rest) = line.trim().split_once(' ').unwrap_or((line.trim(), ""));
    if command.is_empty() {
        return None;
    }
    if command == "say" || command == "tell" {
        let (head, text) = rest.split_once(' ').unwrap_or((rest, ""));
        let args = if head == "place" && !text.is_empty() {
            json!({ "to": { "place": view.place }, "text": text })
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
