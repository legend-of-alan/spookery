//! The human player: claims a PC, turns typed lines into commands, and shows each message as
//! lines or in the game window. For the window's place panel it asks for the map at each new
//! place and keeps that reply out of the transcript. Quitting releases the PC. A dropped
//! connection opens again, takes the PC back by token and asks for the map again. Opens a fresh
//! connection after the PC choice, so a slow choice never trips the first-message deadline. If
//! the host is gone, returns `ExitReason::HostClosed` so the caller can print `host closed` after
//! the terminal is restored.

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
    let Some(pc_choice) = terminal.next_line(None, &commands).await? else {
        return Ok(ExitReason::PlayerLeft);
    };
    let actor_id = pc_choice.trim();
    if actor_id.is_empty() {
        return Err(Error::Message("SCHEMA".into()));
    }
    let mut connection = websocket::Connection::open(addr, "player").await?.0;
    let args = json!({ "actor": actor_id });
    let (mut view, mut token) = claim_pc(&mut connection, args, terminal, None, None).await?;
    let asks_for_map = terminal.has_place_panel() && commands.iter().any(|known| known == "map");
    let mut is_map_due = true;
    let mut map_request_id: Option<String> = None;
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);
    loop {
        let unsent = if asks_for_map && is_map_due && map_request_id.is_none() {
            is_map_due = false;
            match connection.send("map", json!({})).await {
                Ok(request_id) => {
                    map_request_id = Some(request_id);
                    continue;
                }
                Err(Error::Io(_)) => None,
                Err(err) => return Err(err),
            }
        } else {
            tokio::select! {
                message = connection.recv() => match message {
                    Some(Ok(message)) => {
                        if answers(&message, map_request_id.as_deref()) {
                            map_request_id = None;
                            terminal.fill_place_panel(&message, Some(&view))?;
                            continue;
                        }
                        if let ServerBody::View(new_view) = &message.body {
                            is_map_due |= new_view.place != view.place;
                            view = (**new_view).clone();
                        }
                        terminal.show(&message, Some(&view))?;
                        continue;
                    }
                    Some(Err(Error::Io(_))) | None => None,
                    Some(Err(err)) => return Err(err),
                },
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
                        Ok(_) => continue,
                        Err(Error::Io(_)) => Some((command.to_string(), args)),
                        Err(err) => return Err(err),
                    }
                }
            }
        };
        if !reconnect(
            addr,
            &mut connection,
            &mut view,
            &mut token,
            terminal,
            map_request_id.take().as_deref(),
        )
        .await?
        {
            return Ok(ExitReason::HostClosed);
        }
        is_map_due = true;
        if let Some((command, args)) = unsent {
            connection.send(&command, args).await?;
        }
    }
    connection.release().await?;
    Ok(ExitReason::PlayerLeft)
}

/// Opens a connection again and takes the PC back by its token, after the last `seq` received.
/// Replaces the live connection, view and token only when that succeeds. `false` when
/// `spook run` is gone, leaving those live values untouched.
async fn reconnect(
    addr: SocketAddr,
    connection: &mut websocket::Connection,
    view: &mut protocol::View,
    token: &mut String,
    terminal: &mut Terminal,
    lost_map_request_id: Option<&str>,
) -> Result<bool, Error> {
    let mut new_connection = match websocket::Connection::open(addr, "player").await {
        Ok((connection, _)) => connection,
        Err(error) => {
            tracing::debug!(%error, "could not connect to the host again");
            return Ok(false);
        }
    };
    let args = json!({ "token": token.as_str(), "last_seq": terminal.last_seq() });
    let (new_view, reconnect_token) = claim_pc(
        &mut new_connection,
        args,
        terminal,
        Some(view.clone()),
        lost_map_request_id,
    )
    .await?;
    *connection = new_connection;
    *view = new_view;
    *token = reconnect_token;
    Ok(true)
}

/// Sends `claim` with `args` and shows its reply, what it missed, and the first view, all
/// within `websocket::CONNECTION_TIMEOUT`. A missed reply to `lost_map_request_id` goes to the
/// place panel. Returns that view and the token. A refusal ends play.
async fn claim_pc(
    connection: &mut websocket::Connection,
    args: Value,
    terminal: &mut Terminal,
    mut view: Option<protocol::View>,
    lost_map_request_id: Option<&str>,
) -> Result<(protocol::View, String), Error> {
    let claiming = async {
        let request_id = connection.send("claim", args).await?;
        let mut reconnect_token = String::new();
        loop {
            let Some(server_message) = connection.recv().await.transpose()? else {
                return Err(Error::Message("NOT_IN_SPEC".into()));
            };
            if let ServerBody::Refused(refusal) = &server_message.body {
                if answers(&server_message, Some(request_id.as_str())) {
                    return Err(Error::Message(text::render_refusal(
                        refusal,
                        terminal.is_raw(),
                    )));
                }
            }
            let missed_messages = match &server_message.body {
                ServerBody::Claimed {
                    missed_messages, ..
                } => missed_messages.as_slice(),
                _ => &[],
            };
            let messages = std::iter::once(&server_message).chain(missed_messages);
            for message in messages {
                if answers(message, lost_map_request_id) {
                    terminal.fill_place_panel(message, view.as_ref())?;
                    continue;
                }
                if let ServerBody::View(new_view) = &message.body {
                    view = Some((**new_view).clone());
                }
                terminal.show(message, view.as_ref())?;
            }
            match server_message.body {
                ServerBody::Claimed { token, .. } => reconnect_token = token,
                ServerBody::View(new_view) => return Ok((*new_view, reconnect_token)),
                _ => {}
            }
        }
    };
    tokio::time::timeout(websocket::CONNECTION_TIMEOUT, claiming)
        .await
        .unwrap_or_else(|_| Err(Error::Message("Claiming the character timed out.".into())))
}

/// Whether `message` replies to the request `request_id` names.
fn answers(message: &ServerMessage, request_id: Option<&str>) -> bool {
    request_id.is_some() && message.reply_to.as_deref() == request_id
}

/// The command a typed line names, and its arguments. `tell` is the word `say`.
/// `say place <text>` speaks to the place `view` shows. Any other say of two or more words
/// goes to the host as typed, the first word as the actor in `to` and the rest in `text`, and the
/// host finds who it names. Any other first word is a move with the rest of the line as
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
    use std::future::Future;
    use std::time::Duration;

    use futures_util::{SinkExt, StreamExt};
    use serde_json::{json, Value};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
    use tokio_tungstenite::tungstenite::Message;
    use tokio_tungstenite::WebSocketStream;

    use super::{play, ExitReason};
    use crate::protocol::ServerBody;
    use crate::terminal::{SurfaceCall, Terminal};

    const RECORDING: &str = include_str!("../protocol/sessions/v4.jsonl");

    /// How long one test may run before it fails instead of hanging.
    const WAIT_LIMIT: Duration = Duration::from_secs(20);

    /// The body of the first message of `kind` that Rowan got in the recorded docks session.
    fn recorded_body(kind: &str) -> Value {
        let row = (RECORDING.lines())
            .map(|row| serde_json::from_str::<Value>(row).unwrap())
            .find(|row| row["seat"] == "player:rowan" && row["get"]["body"]["kind"] == kind)
            .unwrap();
        row["get"]["body"].clone()
    }

    /// Rowan's recorded view of the Docks Shop, moved to `place_id`.
    fn view_body(place_id: &str, place_name: &str) -> Value {
        let mut body = recorded_body("view");
        body["place"] = place_id.into();
        body["place_name"] = place_name.into();
        body
    }

    fn shop_view() -> Value {
        view_body("docks_shop", "Docks Shop")
    }

    fn street_view() -> Value {
        view_body("docks_street", "Docks Street")
    }

    fn map_body() -> Value {
        json!({ "kind": "map", "places": [] })
    }

    /// One connection the scripted host accepted.
    struct HostConnection(WebSocketStream<TcpStream>);

    impl HostConnection {
        /// Accepts play's next connection and sends the recorded hello.
        async fn accept(listener: &TcpListener) -> Self {
            let (stream, _) = listener.accept().await.unwrap();
            let mut connection = Self(tokio_tungstenite::accept_async(stream).await.unwrap());
            connection.send(None, recorded_body("hello")).await;
            connection
        }

        async fn send(&mut self, reply_to: Option<&str>, body: Value) {
            let message = json!({ "reply_to": reply_to, "body": body });
            self.0
                .send(Message::text(message.to_string()))
                .await
                .unwrap();
        }

        /// Reads the next command play sends, which must be `kind`, and returns its id.
        async fn expect_command(&mut self, kind: &str) -> String {
            loop {
                let Message::Text(text) = self.0.next().await.unwrap().unwrap() else {
                    continue;
                };
                let command: Value = serde_json::from_str(&text).unwrap();
                assert_eq!(command["body"]["kind"], kind, "{command}");
                return command["id"].as_str().unwrap().to_string();
            }
        }

        /// Answers play's `claim` with `claimed`, carrying `missed_messages`, then `view`.
        async fn answer_claim(&mut self, missed_messages: Value, view: Value) {
            let claim_id = self.expect_command("claim").await;
            let mut claimed = recorded_body("claimed");
            claimed["missed_messages"] = missed_messages;
            self.send(Some(&claim_id), claimed).await;
            self.send(None, view).await;
        }

        /// Answers play's `map` with an empty map, and waits until the place panel has it.
        async fn answer_map(&mut self, surface_ends: &mut TestSurfaceEnds) {
            let map_request_id = self.expect_command("map").await;
            self.send(Some(&map_request_id), map_body()).await;
            surface_ends.expect_place_panel_reply(&map_request_id).await;
        }

        /// Has the player send `line`, which must be the command `kind`, answers it with `view`,
        /// and waits until play shows that reply.
        async fn answer_line(
            &mut self,
            surface_ends: &mut TestSurfaceEnds,
            line: &str,
            kind: &str,
            view: Value,
        ) {
            surface_ends.line_sender.send(line.into()).unwrap();
            let request_id = self.expect_command(kind).await;
            let place_id = view["place"].as_str().unwrap().to_string();
            self.send(Some(&request_id), view).await;
            surface_ends.expect_view(&place_id, Some(&request_id)).await;
        }

        async fn answer_release(&mut self) {
            let release_id = self.expect_command("release").await;
            self.send(Some(&release_id), json!({ "kind": "released" }))
                .await;
        }
    }

    /// The test's ends of the test surface: the sender of the lines the player types after
    /// choosing Rowan, and the calls play makes on the surface. Dropping it quits play.
    struct TestSurfaceEnds {
        line_sender: UnboundedSender<String>,
        calls: UnboundedReceiver<SurfaceCall>,
    }

    impl TestSurfaceEnds {
        /// The next call play makes. A `show` of a map fails the test.
        async fn next_call(&mut self) -> SurfaceCall {
            let call = self.calls.recv().await.unwrap();
            if let SurfaceCall::Show(message) = &call {
                let is_map = matches!(message.body, ServerBody::Map { .. });
                assert!(!is_map, "a map reached the transcript: {message:?}");
            }
            call
        }

        /// Skips the messages play shows up to the next place panel reply, which must answer
        /// `request_id`.
        async fn expect_place_panel_reply(&mut self, request_id: &str) {
            loop {
                if let SurfaceCall::FillPlacePanel(reply) = self.next_call().await {
                    assert_eq!(reply.reply_to.as_deref(), Some(request_id));
                    return;
                }
            }
        }

        /// Skips the messages play shows up to the view of `place_id` that answers `request_id`,
        /// or that answers nothing when it is `None`.
        async fn expect_view(&mut self, place_id: &str, request_id: Option<&str>) {
            loop {
                let message = match self.next_call().await {
                    SurfaceCall::Show(message) => message,
                    call => panic!("expected the view of {place_id}, got {call:?}"),
                };
                let is_place =
                    matches!(&message.body, ServerBody::View(view) if view.place == place_id);
                if is_place && message.reply_to.as_deref() == request_id {
                    return;
                }
            }
        }
    }

    /// Runs `play` on the test surface against `host_script`, which gets the host's listener and
    /// the test's ends of the surface, until play leaves.
    async fn play_against<Script: Future<Output = ()>>(
        host_script: impl FnOnce(TcpListener, TestSurfaceEnds) -> Script,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (line_sender, lines) = mpsc::unbounded_channel();
        let (call_sender, calls) = mpsc::unbounded_channel();
        line_sender.send("rowan".into()).unwrap();
        let mut terminal = Terminal::test(lines, call_sender);
        let surface_ends = TestSurfaceEnds { line_sender, calls };
        let playing = async {
            tokio::join!(
                play(addr, &mut terminal),
                host_script(listener, surface_ends)
            )
        };
        let (exit_reason, ()) = (tokio::time::timeout(WAIT_LIMIT, playing).await)
            .expect("play or the host script was still running after WAIT_LIMIT");
        assert!(matches!(exit_reason.unwrap(), ExitReason::PlayerLeft));
    }

    #[tokio::test]
    async fn play_asks_for_the_map_at_each_new_place_and_keeps_the_reply_out_of_the_transcript() {
        play_against(|listener, mut surface_ends| async move {
            drop(HostConnection::accept(&listener).await);
            let mut connection = HostConnection::accept(&listener).await;
            connection.answer_claim(json!([]), shop_view()).await;
            connection.answer_map(&mut surface_ends).await;
            let ends = &mut surface_ends;
            connection
                .answer_line(ends, "look", "look", shop_view())
                .await;
            let travel = "travel Docks Street";
            connection
                .answer_line(ends, travel, "travel", street_view())
                .await;
            connection.answer_map(&mut surface_ends).await;
            drop(surface_ends);
            connection.answer_release().await;
        })
        .await;
    }

    #[tokio::test]
    async fn a_map_lost_with_its_connection_cannot_hide_a_later_reply() {
        play_against(|listener, mut surface_ends| async move {
            drop(HostConnection::accept(&listener).await);
            let mut connection = HostConnection::accept(&listener).await;
            connection.answer_claim(json!([]), shop_view()).await;
            connection.answer_map(&mut surface_ends).await;
            let ends = &mut surface_ends;
            let travel = "travel Docks Street";
            connection
                .answer_line(ends, travel, "travel", street_view())
                .await;
            let lost_map_request_id = connection.expect_command("map").await;
            drop(connection);
            let mut connection = HostConnection::accept(&listener).await;
            connection.answer_claim(json!([]), street_view()).await;
            connection.answer_map(&mut surface_ends).await;
            let ends = &mut surface_ends;
            connection
                .answer_line(ends, "look", "look", street_view())
                .await;
            surface_ends.line_sender.send("look".into()).unwrap();
            let reused_id = connection.expect_command("look").await;
            assert_eq!(reused_id, lost_map_request_id);
            connection.send(Some(&reused_id), street_view()).await;
            surface_ends
                .expect_view("docks_street", Some(&reused_id))
                .await;
            drop(surface_ends);
            connection.answer_release().await;
        })
        .await;
    }

    #[tokio::test]
    async fn a_missed_map_reply_fills_the_place_panel_after_a_reconnect() {
        play_against(|listener, mut surface_ends| async move {
            drop(HostConnection::accept(&listener).await);
            let mut connection = HostConnection::accept(&listener).await;
            connection.answer_claim(json!([]), shop_view()).await;
            surface_ends.expect_view("docks_shop", None).await;
            let lost_map_request_id = connection.expect_command("map").await;
            drop(connection);
            let mut connection = HostConnection::accept(&listener).await;
            let missed_reply = json!({ "reply_to": lost_map_request_id, "body": map_body() });
            connection
                .answer_claim(json!([missed_reply]), shop_view())
                .await;
            let replayed = loop {
                match surface_ends.next_call().await {
                    SurfaceCall::FillPlacePanel(reply) => break reply,
                    SurfaceCall::Show(message) => {
                        let is_view = matches!(message.body, ServerBody::View(_));
                        assert!(
                            !is_view,
                            "the claim's view came before the missed map reply"
                        );
                    }
                }
            };
            assert_eq!(replayed.reply_to, Some(lost_map_request_id));
            connection.answer_map(&mut surface_ends).await;
            drop(surface_ends);
            connection.answer_release().await;
        })
        .await;
    }
}
