//! The human player: claims a PC, turns typed lines into commands, and shows each message as
//! lines or in the game window. Quitting releases the PC. A dropped connection dials again and
//! takes it back by token.

use std::io::{self, Write};
use std::net::SocketAddr;

use serde_json::{json, Value};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::protocol::{self, Reason, Refusal, ServerBody, ServerMessage};
use crate::text;
use crate::window::Window;
use crate::ws::Connection;
use crate::Error;

/// How `spook play` shows the table: plain lines, as text or with `raw` as protocol frames, or
/// the game window.
pub enum Mode {
    Lines { raw: bool },
    Window,
}

/// Plays one sitting in `mode`. Dials a fresh connection after the PC choice, so a slow choice
/// never trips the first-message deadline.
pub async fn run(addr: SocketAddr, mode: Mode) -> Result<(), Error> {
    let mut output = Output::open(mode)?;
    let closing = sit(addr, &mut output).await;
    drop(output);
    if let Ok(Closing::HostClosed) = closing {
        eprintln!("host closed");
    }
    closing.map(drop)
}

/// How a sitting ended.
enum Closing {
    /// The player left, releasing their PC if they held one.
    PlayerLeft,
    /// `spook run` was gone when play dialed it again.
    HostClosed,
}

async fn sit(addr: SocketAddr, output: &mut Output) -> Result<Closing, Error> {
    let (seat, hello) = Connection::open(addr, "player").await?;
    let commands = match &hello.body {
        ServerBody::Hello { commands, .. } => commands.clone(),
        _ => Vec::new(),
    };
    output.show(&hello)?;
    drop(seat);
    let Some(chosen) = output.next_line().await? else {
        return Ok(Closing::PlayerLeft);
    };
    let actor = parse_pc_choice(&chosen)?;
    let mut seat = Connection::open(addr, "player").await?.0;
    let (mut here, mut token) = take_claim(&mut seat, json!({ "actor": actor }), output).await?;
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);
    loop {
        tokio::select! {
            message = seat.recv() => {
                let message = match message {
                    Some(Ok(message)) => message,
                    Some(Err(Error::Io(_))) | None => {
                        let Some(back) = reconnect(addr, &token, output).await? else {
                            return Ok(Closing::HostClosed);
                        };
                        (seat, here, token) = back;
                        continue;
                    }
                    Some(Err(err)) => return Err(err),
                };
                output.show(&message)?;
                if let ServerBody::View(view) = message.body {
                    here = *view;
                }
            }
            _ = &mut ctrl_c => break,
            line = output.next_line() => {
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
                    output.show_body(&ServerBody::Refused(Refusal::from(Reason::NotInSpec)))?;
                    continue;
                }
                match seat.send(command, args.clone()).await {
                    Ok(_) => {}
                    Err(Error::Io(_)) => {
                        let Some(back) = reconnect(addr, &token, output).await? else {
                            return Ok(Closing::HostClosed);
                        };
                        (seat, here, token) = back;
                        seat.send(command, args).await?;
                    }
                    Err(err) => return Err(err),
                }
            }
        }
    }
    seat.release().await?;
    Ok(Closing::PlayerLeft)
}

/// Dials again and takes the PC back by its token, after the last `seq` shown. `None` when
/// `spook run` is gone.
async fn reconnect(
    addr: SocketAddr,
    token: &str,
    output: &mut Output,
) -> Result<Option<(Connection, protocol::View, String)>, Error> {
    let mut seat = match Connection::open(addr, "player").await {
        Ok((seat, _)) => seat,
        Err(error) => {
            tracing::debug!(%error, "could not dial the host again");
            return Ok(None);
        }
    };
    let args = json!({ "token": token, "last_seq": output.last_seq });
    let (here, token) = take_claim(&mut seat, args, output).await?;
    Ok(Some((seat, here, token)))
}

/// Sends `claim` with `args` and shows its reply, what it missed, and the first view.
/// Returns that view and the claim's token. A refused claim ends play.
async fn take_claim(
    seat: &mut Connection,
    args: Value,
    output: &mut Output,
) -> Result<(protocol::View, String), Error> {
    let claim = seat.send("claim", args).await?;
    let mut token = String::new();
    loop {
        let Some(message) = seat.recv().await.transpose()? else {
            return Err(Error::Message("NOT_IN_SPEC".into()));
        };
        if let ServerBody::Refused(refusal) = &message.body {
            if message.reply_to.as_deref() == Some(claim.as_str()) {
                return Err(Error::Message(text::render_refusal(refusal, output.raw())));
            }
        }
        output.show(&message)?;
        match message.body {
            ServerBody::Claimed {
                token: held,
                missed,
                ..
            } => {
                token = held;
                for missed in &missed {
                    output.show(missed)?;
                }
            }
            ServerBody::View(view) => return Ok((*view, token)),
            _ => {}
        }
    }
}

/// Where play shows each message and reads each line the player sends, and the last `seq` it
/// showed, so a reconnect asks for what came after.
struct Output {
    surface: Surface,
    last_seq: u64,
}

/// Plain lines on stdout and stderr with lines typed on stdin, or the game window.
enum Surface {
    Lines {
        printer: Printer,
        typed: UnboundedReceiver<String>,
    },
    Window(Box<Window>),
}

impl Output {
    /// Opens the window in window mode. In lines mode, starts reading stdin, which the window
    /// leaves to its key events.
    fn open(mode: Mode) -> Result<Self, Error> {
        let surface = match mode {
            Mode::Lines { raw } => Surface::Lines {
                printer: Printer {
                    text: text::Renderer::new(),
                    raw,
                },
                typed: crate::stdin_lines(),
            },
            Mode::Window => Surface::Window(Box::new(Window::open()?)),
        };
        Ok(Self {
            surface,
            last_seq: 0,
        })
    }

    fn show(&mut self, message: &ServerMessage) -> Result<(), Error> {
        self.last_seq = self.last_seq.max(message.seq.unwrap_or(0));
        match &mut self.surface {
            Surface::Lines { printer, .. } if printer.raw => {
                printer.show_raw(message);
                Ok(())
            }
            _ => self.show_body(&message.body),
        }
    }

    fn show_body(&mut self, body: &ServerBody) -> Result<(), Error> {
        match &mut self.surface {
            Surface::Lines { printer, .. } => {
                printer.show(body);
                Ok(())
            }
            Surface::Window(window) => window.show(body),
        }
    }

    /// The next line the player sends: typed on stdin, or chosen or typed in the window.
    /// `None` when stdin ends or the player quits the window.
    async fn next_line(&mut self) -> Result<Option<String>, Error> {
        match &mut self.surface {
            Surface::Lines { typed, .. } => Ok(typed.recv().await),
            Surface::Window(window) => window.next_line().await,
        }
    }

    /// Whether refusals show as their bare code.
    fn raw(&self) -> bool {
        matches!(&self.surface, Surface::Lines { printer, .. } if printer.raw)
    }
}

/// What lines mode prints: the picker, each body as text or with `raw` each message as its
/// frame, and refusals on stderr.
struct Printer {
    text: text::Renderer,
    raw: bool,
}

impl Printer {
    fn show(&mut self, body: &ServerBody) {
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
    fn show_raw(&mut self, message: &ServerMessage) {
        match &message.body {
            ServerBody::Hello { .. } | ServerBody::Refused(_) => self.show(&message.body),
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
