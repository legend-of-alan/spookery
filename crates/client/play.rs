//! The human player: claims a PC, turns typed lines into commands, and shows each message as
//! lines or in the game window. Quitting releases the PC. A dropped connection dials again and
//! takes it back by token. Dials a fresh connection after the PC choice, so a slow choice never
//! trips the first-message deadline. If the host is gone, prints `host closed` after the terminal
//! is restored.

use std::io::{self, Write};
use std::net::SocketAddr;

use serde_json::{json, Value};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::protocol::{self, Reason, Refusal, ServerBody, ServerMessage};
use crate::text;
use crate::window::Window;
use crate::ws;
use crate::Error;

/// Opens the game window, connects, claims a PC, and plays until the player leaves or the host
/// closes.
pub async fn run_window(addr: SocketAddr) -> Result<(), Error> {
    run(addr, Terminal::window()?).await
}

/// Prints plain lines, as text or with `raw` as protocol frames, connects, claims a PC, and plays
/// until the player leaves or the host closes.
pub async fn run_lines(addr: SocketAddr, raw: bool) -> Result<(), Error> {
    run(addr, Terminal::lines(raw)).await
}

async fn run(addr: SocketAddr, mut terminal: Terminal) -> Result<(), Error> {
    let exit = join(addr, &mut terminal).await;
    drop(terminal);
    if let Ok(Exit::HostClosed) = exit {
        eprintln!("host closed");
    }
    exit.map(drop)
}

/// How play exited.
enum Exit {
    /// The player left, releasing their PC if they held one.
    PlayerLeft,
    /// `spook run` was gone when play dialed it again.
    HostClosed,
}

async fn join(addr: SocketAddr, terminal: &mut Terminal) -> Result<Exit, Error> {
    let (connection, hello) = ws::Connection::open(addr, "player").await?;
    let commands = match &hello.body {
        ServerBody::Hello { commands, .. } => commands.clone(),
        _ => Vec::new(),
    };
    terminal.show(&hello)?;
    drop(connection);
    let Some(chosen) = terminal.next_line().await? else {
        return Ok(Exit::PlayerLeft);
    };
    let actor = parse_pc_choice(&chosen)?;
    let mut connection = ws::Connection::open(addr, "player").await?.0;
    let (mut here, mut token) = claim(&mut connection, json!({ "actor": actor }), terminal).await?;
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);
    loop {
        tokio::select! {
            message = connection.recv() => {
                let message = match message {
                    Some(Ok(message)) => message,
                    Some(Err(Error::Io(_))) | None => {
                        if !reconnect_into(addr, &mut connection, &mut here, &mut token, terminal)
                            .await?
                        {
                            return Ok(Exit::HostClosed);
                        }
                        continue;
                    }
                    Some(Err(err)) => return Err(err),
                };
                terminal.show(&message)?;
                if let ServerBody::View(view) = message.body {
                    here = *view;
                }
            }
            _ = &mut ctrl_c => break,
            line = terminal.next_line() => {
                let Some(line) = line? else {
                    break;
                };
                if line.trim() == "release" {
                    break;
                }
                let Some((command, args)) = parse_command(&line, &here) else {
                    continue;
                };
                if !commands.iter().any(|known| known == command) {
                    terminal.show_body(&ServerBody::Refused(Refusal::from(Reason::NotInSpec)))?;
                    continue;
                }
                match connection.send(command, args.clone()).await {
                    Ok(_) => {}
                    Err(Error::Io(_)) => {
                        if !reconnect_into(addr, &mut connection, &mut here, &mut token, terminal)
                            .await?
                        {
                            return Ok(Exit::HostClosed);
                        }
                        connection.send(command, args).await?;
                    }
                    Err(err) => return Err(err),
                }
            }
        }
    }
    connection.release().await?;
    Ok(Exit::PlayerLeft)
}

/// Dials again and takes the PC back by its token, after the last `seq` shown. `None` when
/// `spook run` is gone.
async fn reconnect(
    addr: SocketAddr,
    token: &str,
    terminal: &mut Terminal,
) -> Result<Option<(ws::Connection, protocol::View, String)>, Error> {
    let mut connection = match ws::Connection::open(addr, "player").await {
        Ok((connection, _)) => connection,
        Err(error) => {
            tracing::debug!(%error, "could not dial the host again");
            return Ok(None);
        }
    };
    let args = json!({ "token": token, "last_seq": terminal.last_seq });
    let (here, token) = claim(&mut connection, args, terminal).await?;
    Ok(Some((connection, here, token)))
}

/// Puts a successful `reconnect` onto the live connection, view and token. `false` when
/// `spook run` is gone.
async fn reconnect_into(
    addr: SocketAddr,
    connection: &mut ws::Connection,
    here: &mut protocol::View,
    token: &mut String,
    terminal: &mut Terminal,
) -> Result<bool, Error> {
    let Some(back) = reconnect(addr, token, terminal).await? else {
        return Ok(false);
    };
    (*connection, *here, *token) = back;
    Ok(true)
}

/// Sends `claim` with `args` and shows its reply, what it missed, and the first view.
/// Returns that view and the token. If the table refuses, play ends.
async fn claim(
    connection: &mut ws::Connection,
    args: Value,
    terminal: &mut Terminal,
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
        terminal.show(&message)?;
        match message.body {
            ServerBody::Claimed {
                token: held,
                missed,
                ..
            } => {
                token = held;
                for missed in &missed {
                    terminal.show(missed)?;
                }
            }
            ServerBody::View(view) => return Ok((*view, token)),
            _ => {}
        }
    }
}

/// Where play shows each message and reads each line the player sends, and the last `seq` it
/// showed, so a reconnect asks for what came after.
struct Terminal {
    surface: Surface,
    last_seq: u64,
}

/// Plain lines on stdout and stderr with lines typed on stdin, or the game window.
enum Surface {
    Lines(Lines),
    Window(Box<Window>),
}

impl Terminal {
    fn window() -> Result<Self, Error> {
        Ok(Self {
            surface: Surface::Window(Box::new(Window::open()?)),
            last_seq: 0,
        })
    }

    fn lines(raw: bool) -> Self {
        Self {
            surface: Surface::Lines(Lines {
                text: text::Renderer::new(),
                raw,
                typed: crate::stdin_lines(),
            }),
            last_seq: 0,
        }
    }

    fn show(&mut self, message: &ServerMessage) -> Result<(), Error> {
        self.last_seq = self.last_seq.max(message.seq.unwrap_or(0));
        match &mut self.surface {
            Surface::Lines(lines) if lines.raw => {
                lines.print_raw(message);
                Ok(())
            }
            _ => self.show_body(&message.body),
        }
    }

    fn show_body(&mut self, body: &ServerBody) -> Result<(), Error> {
        match &mut self.surface {
            Surface::Lines(lines) => {
                lines.print(body);
                Ok(())
            }
            Surface::Window(window) => window.show(body),
        }
    }

    /// The next line the player sends: typed on stdin, or chosen or typed in the window.
    /// `None` when stdin ends or the player quits the window.
    async fn next_line(&mut self) -> Result<Option<String>, Error> {
        match &mut self.surface {
            Surface::Lines(lines) => Ok(lines.typed.recv().await),
            Surface::Window(window) => window.next_line().await,
        }
    }

    /// Whether refusals show as their bare code.
    fn is_raw(&self) -> bool {
        matches!(&self.surface, Surface::Lines(lines) if lines.raw)
    }
}

/// The plain-lines surface. Game text goes to stdout and refusals go to stderr. With `raw`,
/// each message other than the picker or a refusal is its protocol frame.
struct Lines {
    text: text::Renderer,
    raw: bool,
    typed: UnboundedReceiver<String>,
}

impl Lines {
    fn print(&mut self, body: &ServerBody) {
        let shown = match body {
            ServerBody::Hello {
                open,
                taken,
                pc_names,
                ..
            } => text::render_picker(&text::picker_rows(open, taken, pc_names)),
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
