//! The human GM: commands in, tool calls out.

use std::collections::VecDeque;
use std::io::{self, Write};
use std::net::SocketAddr;

use rmcp::model::ErrorCode;
use serde::Serialize;
use serde_json::json;

use crate::door::{event_line, GmClient, Incoming};
use crate::protocol::{GmEvent, IntentCall, PlayerFrame, Refusal, SayArgs};
pub use crate::text::actor_name;
use crate::text::{self, render_checks, render_dice, render_diff, render_event, render_status};
use crate::Error;

const HELP: &str = "\
Commands:
  status
      Who is seated, and the queue.
  look <actor>
      Perceive as that actor. A pack id, or player:<id>, or npc:<id>.
  say <text>
      Speak to the whole table.
  say @<place> <text>
      Speak to one place.
  say #<actor> <text>
      Speak to one actor. The GM speaks as the GM.
  queue <actor> <move> [target]
      Queue a move. A target is a place id or an actor id.
  commit
      Commit the queue as one round.
  help
      Show these commands.
  quit
      Leave the seat.
An unknown command prints this list and sends nothing.";

pub async fn run(door: SocketAddr, raw: bool) -> Result<(), Error> {
    let client = match GmClient::connect(door).await {
        Ok(client) => client,
        Err(Error::Refused(refusal)) => {
            let error = json!({
                "code": ErrorCode::INTERNAL_ERROR,
                "message": refusal.to_string(),
                "data": &refusal,
            });
            let line = json!({ "jsonrpc": "2.0", "id": 1, "error": error }).to_string();
            return Err(Error::Message(echo_refusal(&refusal, &line, raw)));
        }
        Err(err) => return Err(err),
    };
    let mut session = Session {
        client,
        is_raw: raw,
        next_id: 2,
        printed_checks: PrintedChecks::default(),
    };
    if raw {
        let version = session.client.protocol_version();
        let result = json!({ "protocolVersion": version });
        emit(&json!({ "jsonrpc": "2.0", "id": 1, "result": result }).to_string());
    }
    emit(HELP);
    let mut input = crate::stdin_lines();
    let result = async {
        loop {
            tokio::select! {
                incoming = session.client.next_incoming() => {
                    if !session.show_incoming(incoming) {
                        return Ok(());
                    }
                }
                line = input.recv() => match line {
                    Some(line) if session.on_line(&line).await? => {}
                    _ => return Ok(()),
                },
            }
        }
    }
    .await;
    session.client.close().await;
    result
}

struct Session {
    client: GmClient,
    is_raw: bool,
    /// The id of the next result line raw mode prints. The initialize line is 1.
    next_id: u64,
    printed_checks: PrintedChecks,
}

/// Check lines `commit` already printed. Each matching event is skipped once,
/// so a later check still prints. Events that arrive after the result use
/// this list. Events that arrive before it are matched in `show_arrived`.
#[derive(Debug, Default)]
struct PrintedChecks(VecDeque<String>);

impl PrintedChecks {
    /// The check lines one commit result printed. Replaces any older lines.
    fn replace(&mut self, lines: Vec<String>) {
        self.0 = lines.into();
    }

    /// True when `line` is the next check event of the last commit.
    fn skip(&mut self, line: &str) -> bool {
        if self.0.front().map(String::as_str) == Some(line) {
            self.0.pop_front();
            return true;
        }
        self.0.clear();
        false
    }

    /// A later result came in, so the last commit's events were all read.
    fn clear(&mut self) {
        self.0.clear();
    }
}

impl Session {
    async fn on_line(&mut self, line: &str) -> Result<bool, Error> {
        match parse_command(line) {
            Parsed::Empty => Ok(true),
            Parsed::Unknown => {
                emit(HELP);
                Ok(true)
            }
            Parsed::Command(command) => self.on_command(command).await,
        }
    }

    async fn on_command(&mut self, command: Command) -> Result<bool, Error> {
        match command {
            Command::Help => emit(HELP),
            Command::Quit => return Ok(false),
            Command::Status => self.on_status().await?,
            Command::Look { actor } => self.on_look(&actor).await?,
            Command::Say(address) => self.on_say(address).await?,
            Command::Queue {
                actor,
                move_name,
                target,
            } => self.on_queue(&actor, &move_name, target.as_deref()).await?,
            Command::Commit => self.on_commit().await?,
        }
        Ok(true)
    }

    async fn on_status(&mut self) -> Result<(), Error> {
        let reply = self.client.table_status(None).await;
        self.show_result(reply, &[], |_session, status| {
            emit(&render_status(&status));
        })
    }

    async fn on_look(&mut self, actor: &str) -> Result<(), Error> {
        let reply = self.client.perceive(actor).await;
        // The table sends the view as the player `view` frame, `kind` included.
        let reply = reply.map(|reply| reply.map(|view| PlayerFrame::View(Box::new(view))));
        self.show_result(reply, &[], |_session, frame| {
            emit(&text::render(&frame));
        })
    }

    async fn on_say(&mut self, address: SayArgs) -> Result<(), Error> {
        let reply = self.client.say(&address).await;
        // The table answers with the say event, `kind` included.
        let reply = reply.map(|reply| reply.map(GmEvent::Say));
        self.show_result(reply, &[], |_session, _said| {})
    }

    async fn on_queue(
        &mut self,
        actor: &str,
        move_name: &str,
        target: Option<&str>,
    ) -> Result<(), Error> {
        let shown = format!("{} queued: {move_name}", actor_name(actor));
        let call = IntentCall {
            id: None,
            round_index: None,
            actor_id: Some(actor.to_string()),
            move_name: Some(move_name.to_string()),
            target: target.map(str::to_string),
            message_to: None,
            rationale_code: String::new(),
        };
        let reply = self.client.queue_intent(&call).await;
        self.show_result(reply, &[], move |_session, _queued| {
            emit(&shown);
        })
    }

    async fn on_commit(&mut self) -> Result<(), Error> {
        let reply = self.client.commit_round().await;
        // This commit's check events can arrive before or after the tool
        // result. The result text prints each roll. A matching event on
        // either side is shown once. Raw mode and a failed read show none.
        let checks = match &reply {
            Ok(Ok(body)) if !self.is_raw => render_checks(body),
            _ => Vec::new(),
        };
        let printed = checks.clone();
        self.show_result(reply, &checks, move |session, body| {
            emit(&render_diff(&body));
            for line in &printed {
                emit(line);
            }
            for line in &render_dice(&body) {
                emit(line);
            }
            session.printed_checks.replace(printed);
        })
    }

    /// Show one tool reply after the events that came before it, then anything that
    /// arrived since. Raw mode prints the reply's result line, text mode `render`s it.
    /// `checks` are rolls `render` prints, so a matching check event is not printed
    /// again. A refusal or a reply that could not be read is shown instead; a call that
    /// failed prints its message alone.
    fn show_result<T: Serialize>(
        &mut self,
        reply: Result<Result<T, Refusal>, Error>,
        checks: &[String],
        render: impl FnOnce(&mut Self, T),
    ) -> Result<(), Error> {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        let reply = match reply {
            Ok(reply) => Ok(reply),
            Err(Error::Unreadable(body)) => Err(body),
            Err(Error::Io(err)) => return Err(Error::Io(err)),
            Err(err) => {
                if self.is_raw {
                    emit(&err.to_string());
                } else {
                    eprintln!("{err}");
                    let _ = io::stderr().flush();
                }
                return Ok(());
            }
        };
        self.show_arrived(checks);
        match reply {
            Ok(Err(refusal)) => self.show_refusal(&refusal, &result_line(id, &refusal)),
            Ok(Ok(body)) if self.is_raw => emit(&result_line(id, &body)),
            Ok(Ok(body)) => render(self, body),
            Err(body) if self.is_raw => emit(&result_line(id, &body)),
            Err(_) => report_unreadable(),
        }
        self.show_ready();
        Ok(())
    }

    /// Events buffered before this result. `checks` are rolls this result
    /// will print; a matching check event is not printed again.
    fn show_arrived(&mut self, checks: &[String]) {
        let mut pending: VecDeque<&str> = checks.iter().map(String::as_str).collect();
        let events = self.client.take_events();
        for event in &events {
            if let GmEvent::Check(_) = event {
                let line = render_event(event);
                if pending.front().copied() == Some(line.as_str()) {
                    pending.pop_front();
                    continue;
                }
            }
            self.show_event(event);
        }
        // Every check event of the previous commit came before this result.
        self.printed_checks.clear();
    }

    fn show_ready(&mut self) {
        let ready = self.client.drain_ready();
        for incoming in ready {
            let _ = self.show_incoming(incoming);
        }
    }

    fn show_incoming(&mut self, incoming: Incoming) -> bool {
        match incoming {
            Incoming::Event(event) => self.show_event(&event),
            Incoming::Unreadable(line) => {
                eprintln!("{line}");
                let _ = io::stderr().flush();
            }
            Incoming::Eof => return false,
        }
        true
    }

    fn show_event(&mut self, event: &GmEvent) {
        if self.is_raw {
            emit(&event_line(event));
            return;
        }
        let line = render_event(event);
        if matches!(event, GmEvent::Check(_)) && self.printed_checks.skip(&line) {
            return;
        }
        emit(&line);
    }

    fn show_refusal(&self, refusal: &Refusal, raw: &str) {
        eprintln!("{}", echo_refusal(refusal, raw, self.is_raw));
        let _ = io::stderr().flush();
    }
}

/// Raw mode echoes the frame and names the code. Text mode names the refusal.
fn echo_refusal(refusal: &Refusal, raw: &str, raw_mode: bool) -> String {
    if !raw_mode {
        return refusal.to_string();
    }
    emit(raw);
    refusal.code.as_str().to_string()
}

fn emit(line: &str) {
    println!("{line}");
    let _ = io::stdout().flush();
}

/// A tool reply as the JSON-RPC result line raw mode prints.
fn result_line(id: u64, body: &impl Serialize) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "result": { "structuredContent": body } }).to_string()
}

fn report_unreadable() {
    eprintln!("The reply could not be read.");
    let _ = io::stderr().flush();
}

#[derive(Debug)]
enum Command {
    Status,
    Look {
        actor: String,
    },
    Say(SayArgs),
    Queue {
        actor: String,
        move_name: String,
        target: Option<String>,
    },
    Commit,
    Help,
    Quit,
}

enum Parsed {
    Empty,
    Command(Command),
    Unknown,
}

fn parse_command(line: &str) -> Parsed {
    let line = line.trim();
    if line.is_empty() {
        return Parsed::Empty;
    }
    let (head, rest) = split_word(line);
    match head {
        "status" if rest.is_empty() => Parsed::Command(Command::Status),
        "commit" if rest.is_empty() => Parsed::Command(Command::Commit),
        "help" if rest.is_empty() => Parsed::Command(Command::Help),
        "quit" if rest.is_empty() => Parsed::Command(Command::Quit),
        "look" => {
            if rest.is_empty() || rest.split_whitespace().nth(1).is_some() {
                Parsed::Unknown
            } else {
                Parsed::Command(Command::Look {
                    actor: rest.to_string(),
                })
            }
        }
        "say" => parse_say(rest),
        "queue" => parse_queue(rest),
        _ => Parsed::Unknown,
    }
}

fn parse_say(rest: &str) -> Parsed {
    if rest.is_empty() {
        return Parsed::Unknown;
    }
    if let Some(rest) = rest.strip_prefix('@') {
        let (place, text) = split_word(rest.trim());
        if place.is_empty() || text.is_empty() {
            return Parsed::Unknown;
        }
        return Parsed::Command(Command::Say(SayArgs::Place {
            place: place.to_string(),
            text: text.to_string(),
        }));
    }
    if let Some(rest) = rest.strip_prefix('#') {
        let (actor, text) = split_word(rest.trim());
        if actor.is_empty() || text.is_empty() {
            return Parsed::Unknown;
        }
        return Parsed::Command(Command::Say(SayArgs::Actor {
            actor: actor.to_string(),
            text: text.to_string(),
        }));
    }
    Parsed::Command(Command::Say(SayArgs::Table {
        text: rest.to_string(),
    }))
}

fn parse_queue(rest: &str) -> Parsed {
    let mut parts = rest.split_whitespace();
    let Some(actor) = parts.next() else {
        return Parsed::Unknown;
    };
    let Some(move_name) = parts.next() else {
        return Parsed::Unknown;
    };
    let target = parts.next().map(str::to_string);
    if parts.next().is_some() {
        return Parsed::Unknown;
    }
    Parsed::Command(Command::Queue {
        actor: actor.to_string(),
        move_name: move_name.to_string(),
        target,
    })
}

fn split_word(text: &str) -> (&str, &str) {
    let text = text.trim();
    match text.split_once(char::is_whitespace) {
        Some((head, tail)) => (head, tail.trim()),
        None => (text, ""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::render_check;

    #[test]
    fn a_commit_skips_its_own_check_events_once() {
        let roll = render_check(&[4, 2], 1, 7, 0, "success");
        let mut printed = PrintedChecks::default();
        printed.replace(vec![roll.clone()]);
        assert!(printed.skip(&roll));
        // A later check with the same faces is a new roll and still prints.
        assert!(!printed.skip(&roll));
        assert!(!printed.skip(&roll));
    }

    #[test]
    fn checks_after_a_new_result_are_not_skipped() {
        let roll = render_check(&[3, 3], 0, 6, 0, "success");
        let mut printed = PrintedChecks::default();
        printed.replace(vec![roll.clone()]);
        printed.clear();
        assert!(!printed.skip(&roll));
    }

    #[test]
    fn a_check_that_is_not_next_ends_the_skip() {
        let first = render_check(&[1, 1], 0, 2, 7, "miss");
        let second = render_check(&[6, 6], 0, 12, 7, "success");
        let mut printed = PrintedChecks::default();
        printed.replace(vec![first.clone(), second.clone()]);
        assert!(!printed.skip(&second));
        assert!(!printed.skip(&first));
    }

    #[test]
    fn a_commit_with_two_checks_skips_both_in_order() {
        let first = render_check(&[1, 2], 0, 3, 7, "miss");
        let second = render_check(&[5, 6], 0, 11, 7, "success");
        let mut printed = PrintedChecks::default();
        printed.replace(vec![first.clone(), second.clone()]);
        assert!(printed.skip(&first));
        assert!(printed.skip(&second));
        assert!(!printed.skip(&first));
    }

    fn command(line: &str) -> Command {
        match parse_command(line) {
            Parsed::Command(command) => command,
            Parsed::Empty => panic!("empty"),
            Parsed::Unknown => panic!("unknown"),
        }
    }

    #[test]
    fn each_command_parses() {
        assert!(matches!(command("status"), Command::Status));
        assert!(matches!(command("commit"), Command::Commit));
        assert!(matches!(command("help"), Command::Help));
        assert!(matches!(command("quit"), Command::Quit));
        match command("look pc_rowan") {
            Command::Look { actor } => assert_eq!(actor, "pc_rowan"),
            other => panic!("{other:?}"),
        }
        match command("say good evening") {
            Command::Say(SayArgs::Table { text }) => assert_eq!(text, "good evening"),
            other => panic!("{other:?}"),
        }
        match command("say @docks_shop the lamp is lit") {
            Command::Say(SayArgs::Place { place, text }) => {
                assert_eq!(place, "docks_shop");
                assert_eq!(text, "the lamp is lit");
            }
            other => panic!("{other:?}"),
        }
        match command("say #mira come in") {
            Command::Say(SayArgs::Actor { actor, text }) => {
                assert_eq!(actor, "mira");
                assert_eq!(text, "come in");
            }
            other => panic!("{other:?}"),
        }
        match command("queue mira restock_lamp_oil") {
            Command::Queue {
                actor,
                move_name,
                target,
            } => {
                assert_eq!(actor, "mira");
                assert_eq!(move_name, "restock_lamp_oil");
                assert!(target.is_none());
            }
            other => panic!("{other:?}"),
        }
        match command("queue pc_rowan travel docks_street") {
            Command::Queue {
                actor,
                move_name,
                target,
            } => {
                assert_eq!(actor, "pc_rowan");
                assert_eq!(move_name, "travel");
                assert_eq!(target.as_deref(), Some("docks_street"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_unknown_line_sends_nothing() {
        assert!(matches!(parse_command(""), Parsed::Empty));
        assert!(matches!(parse_command("   "), Parsed::Empty));
        assert!(matches!(parse_command("dance"), Parsed::Unknown));
        assert!(matches!(parse_command("status now"), Parsed::Unknown));
        assert!(matches!(parse_command("look"), Parsed::Unknown));
        assert!(matches!(parse_command("look a b"), Parsed::Unknown));
        assert!(matches!(parse_command("say"), Parsed::Unknown));
        assert!(matches!(parse_command("say @docks_shop"), Parsed::Unknown));
        assert!(matches!(parse_command("say #mira"), Parsed::Unknown));
        assert!(matches!(parse_command("queue mira"), Parsed::Unknown));
        assert!(matches!(
            parse_command("queue mira travel docks_street extra"),
            Parsed::Unknown
        ));
    }
}
