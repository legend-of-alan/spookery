//! The human player: claims a PC, turns typed lines into commands, and shows each message as
//! lines or in the game window. Quitting releases the PC. A dropped connection opens again and
//! takes it back by token. Opens a fresh connection after the PC choice, so a slow choice never
//! trips the first-message deadline. If the host is gone, prints `host closed` after the terminal
//! is restored.

use std::io::{self, Write};
use std::net::SocketAddr;

use serde_json::{json, Value};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::protocol::{self, Reason, Refusal, ServerBody, ServerMessage};
use crate::text;
use crate::websocket;
use crate::window::Window;
use crate::Error;

/// Opens the game window, connects, claims a PC, and plays until the player leaves or the host
/// closes.
pub async fn run_windowed(addr: SocketAddr) -> Result<(), Error> {
    run(addr, Terminal::window()?).await
}

/// Prints plain lines, as text or with `raw` as protocol frames, connects, claims a PC, and plays
/// until the player leaves or the host closes.
pub async fn run_plain(addr: SocketAddr, raw: bool) -> Result<(), Error> {
    run(addr, Terminal::plain(raw)).await
}

async fn run(addr: SocketAddr, mut terminal: Terminal) -> Result<(), Error> {
    let exit = join_session(addr, &mut terminal).await;
    drop(terminal);
    if let Ok(ExitReason::HostClosed) = exit {
        eprintln!("host closed");
    }
    exit.map(drop)
}

/// How play exited.
enum ExitReason {
    /// The player left, releasing their PC if they held one.
    PlayerLeft,
    /// `spook run` was gone when play opened a connection again.
    HostClosed,
}

async fn join_session(addr: SocketAddr, terminal: &mut Terminal) -> Result<ExitReason, Error> {
    let (connection, hello) = websocket::Connection::open(addr, "player").await?;
    let commands = match &hello.body {
        ServerBody::Hello { commands, .. } => commands
            .iter()
            .filter(|command| *command != "claim" && *command != "release")
            .cloned()
            .collect(),
        _ => Vec::new(),
    };
    let mut view = None;
    show(terminal, &hello, &mut view)?;
    drop(connection);
    let Some(chosen) = terminal.next_line(view.as_ref(), &commands).await? else {
        return Ok(ExitReason::PlayerLeft);
    };
    let actor = parse_pc_choice(&chosen)?;
    let mut connection = websocket::Connection::open(addr, "player").await?.0;
    let (mut view, mut token) = claim_pc(
        &mut connection,
        json!({ "actor": actor }),
        terminal,
        &mut view,
    )
    .await?;
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
                    terminal.show_body(&ServerBody::Refused(Refusal::from(Reason::NotInSpec)), Some(&view))?;
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
    let args = json!({ "token": token.as_str(), "last_seq": terminal.last_seq });
    let mut current = Some(view.clone());
    let (new_view, new_token) = claim_pc(&mut opened, args, terminal, &mut current).await?;
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
    view: &mut Option<protocol::View>,
) -> Result<(protocol::View, String), Error> {
    let request_id = connection.send("claim", args).await?;
    let mut token = String::new();
    loop {
        let Some(message) = connection.recv().await.transpose()? else {
            return Err(Error::Message("NOT_IN_SPEC".into()));
        };
        if let ServerBody::Refused(refusal) = &message.body {
            if message.reply_to.as_deref() == Some(request_id.as_str()) {
                return Err(Error::Message(text::render_refusal(
                    refusal,
                    terminal.is_raw(),
                )));
            }
        }
        show(terminal, &message, view)?;
        match message.body {
            ServerBody::Claimed {
                token: held,
                missed,
                ..
            } => {
                token = held;
                for missed in &missed {
                    show(terminal, missed, view)?;
                }
            }
            ServerBody::View(body) => return Ok((*body, token)),
            _ => {}
        }
    }
}

fn show(
    terminal: &mut Terminal,
    message: &ServerMessage,
    view: &mut Option<protocol::View>,
) -> Result<(), Error> {
    if let ServerBody::View(body) = &message.body {
        *view = Some((**body).clone());
    }
    terminal.show(message, view.as_ref())
}

/// Where play shows each message and reads each line the player sends, and the last `seq` it
/// showed, so a reconnect asks for what came after.
struct Terminal {
    surface: Surface,
    last_seq: u64,
}

/// Plain lines on stdout and stderr with lines typed on stdin, or the game window.
enum Surface {
    Plain(Plain),
    Window(Box<Window>),
}

impl Terminal {
    fn window() -> Result<Self, Error> {
        Ok(Self {
            surface: Surface::Window(Box::new(Window::open()?)),
            last_seq: 0,
        })
    }

    fn plain(raw: bool) -> Self {
        Self {
            surface: Surface::Plain(Plain {
                text: text::Renderer::new(),
                raw,
                typed: crate::stdin_lines(),
            }),
            last_seq: 0,
        }
    }

    fn show(
        &mut self,
        message: &ServerMessage,
        view: Option<&protocol::View>,
    ) -> Result<(), Error> {
        self.last_seq = self.last_seq.max(message.seq.unwrap_or(0));
        match &mut self.surface {
            Surface::Plain(plain) if plain.raw => {
                plain.print_raw(message);
                Ok(())
            }
            _ => self.show_body(&message.body, view),
        }
    }

    fn show_body(&mut self, body: &ServerBody, view: Option<&protocol::View>) -> Result<(), Error> {
        match &mut self.surface {
            Surface::Plain(plain) => {
                plain.print(body);
                Ok(())
            }
            Surface::Window(window) => window.show(body, view),
        }
    }

    /// The next line the player sends: typed on stdin, or chosen or typed in the window.
    /// `None` when stdin ends or the player quits the window.
    async fn next_line(
        &mut self,
        view: Option<&protocol::View>,
        commands: &[String],
    ) -> Result<Option<String>, Error> {
        match &mut self.surface {
            Surface::Plain(plain) => Ok(plain.typed.recv().await),
            Surface::Window(window) => window.next_line(view, commands).await,
        }
    }

    /// Whether refusals show as their bare code.
    fn is_raw(&self) -> bool {
        matches!(&self.surface, Surface::Plain(plain) if plain.raw)
    }
}

/// The plain surface. Game text goes to stdout and refusals go to stderr. With `raw`,
/// each message other than the picker or a refusal is its protocol frame.
struct Plain {
    text: text::Renderer,
    raw: bool,
    typed: UnboundedReceiver<String>,
}

impl Plain {
    fn print(&mut self, body: &ServerBody) {
        let shown = match body {
            ServerBody::Hello {
                open,
                taken,
                pc_names,
                ..
            } => text::render_pc_picker(&text::pc_picker_rows(open, taken, pc_names)),
            ServerBody::Refused(refusal) => {
                eprintln!("{}", text::render_refusal(refusal, self.raw));
                return;
            }
            _ => self.text.render(body),
        };
        if !shown.is_empty() {
            print_text(&shown);
        }
    }

    /// The picker as text, a refusal as its code on stderr, and any other message as its frame.
    fn print_raw(&mut self, message: &ServerMessage) {
        match &message.body {
            ServerBody::Hello { .. } | ServerBody::Refused(_) => self.print(&message.body),
            _ => print_text(&message.to_text()),
        }
    }
}

fn print_text(text: &str) {
    println!("{text}");
    let _ = io::stdout().flush();
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
