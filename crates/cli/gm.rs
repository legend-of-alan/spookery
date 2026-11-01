//! The human GM: commands in, tool calls out.

use std::collections::VecDeque;
use std::io::{self, Write};
use std::net::SocketAddr;

use rmcp::model::ErrorCode;
use serde::Serialize;
use serde_json::json;

use crate::mcp::{event_line, GmClient};
use crate::protocol::{self, GmEvent, PlayerFrame, Refusal, SayArgs};
use crate::text::{
    self, humanize, render_checks, render_dice, render_diff, render_event, render_status,
};
use crate::Error;

const HELP: &str = "\
Commands:
  status
      Who is seated, and the queue.
  look <actor>
      Perceive as that actor. A bare actor id.
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

pub async fn run(mcp_addr: SocketAddr, raw: bool) -> Result<(), Error> {
    let client = match GmClient::connect(mcp_addr).await {
        Ok(client) => client,
        Err(Error::Refused(refusal)) => {
            if raw {
                let error = json!({
                    "code": ErrorCode::INTERNAL_ERROR,
                    "message": refusal.to_string(),
                    "data": &refusal,
                });
                emit(&json!({ "jsonrpc": "2.0", "id": 1, "error": error }).to_string());
            }
            return Err(Error::Message(refusal_text(&refusal, raw)));
        }
        Err(err) => return Err(err),
    };
    let mut session = Session {
        client,
        raw,
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
                received = session.client.recv() => match received {
                    Some(received) => session.show_received(received),
                    None => return Ok(()),
                },
                line = input.recv() => match line {
                    Some(line) if session.handle_line(&line).await? => {}
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
    raw: bool,
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
    /// Run one typed line. An empty line does nothing; a line that is not a command prints
    /// `HELP`, as `help` does. False means leave the seat.
    async fn handle_line(&mut self, line: &str) -> Result<bool, Error> {
        if line.trim().is_empty() {
            return Ok(true);
        }
        self.dispatch(parse_command(line).unwrap_or(Command::Help))
            .await
    }

    async fn dispatch(&mut self, command: Command) -> Result<bool, Error> {
        match command {
            Command::Help => emit(HELP),
            Command::Quit => return Ok(false),
            Command::Status => self.status().await?,
            Command::Look { actor } => self.look(&actor).await?,
            Command::Say(args) => self.say(args).await?,
            Command::Queue {
                actor,
                move_name,
                target,
            } => self.queue(&actor, &move_name, target.as_deref()).await?,
            Command::Commit => self.commit().await?,
        }
        Ok(true)
    }

    async fn status(&mut self) -> Result<(), Error> {
        let reply = self.client.table_status(None).await;
        self.show_result(reply, &[], |_session, status| {
            emit(&render_status(&status));
        })
    }

    async fn look(&mut self, actor: &str) -> Result<(), Error> {
        let reply = self.client.perceive(actor).await;
        // The table sends the view as the player `view` frame, `kind` included.
        let reply = reply.map(|reply| reply.map(|view| PlayerFrame::View(Box::new(view))));
        self.show_result(reply, &[], |_session, frame| {
            emit(&text::render(&frame));
        })
    }

    async fn say(&mut self, args: SayArgs) -> Result<(), Error> {
        let reply = self.client.say(&args).await;
        // The table answers with the say event, `kind` included.
        let reply = reply.map(|reply| reply.map(GmEvent::Say));
        self.show_result(reply, &[], |_session, _said| {})
    }

    async fn queue(
        &mut self,
        actor: &str,
        move_name: &str,
        target: Option<&str>,
    ) -> Result<(), Error> {
        let shown = format!("{} queued: {move_name}", humanize(actor));
        let call = protocol::IntentArgs {
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

    async fn commit(&mut self) -> Result<(), Error> {
        let reply = self.client.commit_round().await;
        // This commit's check events can arrive before or after the tool
        // result. The result text prints each roll. A matching event on
        // either side is shown once. Raw mode and a failed read show none.
        let checks = match &reply {
            Ok(Ok(body)) if !self.raw => render_checks(body),
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
                if self.raw {
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
            Ok(Ok(body)) if self.raw => emit(&result_line(id, &body)),
            Ok(Ok(body)) => render(self, body),
            Err(body) if self.raw => emit(&result_line(id, &body)),
            Err(_) => report_unreadable(),
        }
        self.show_ready();
        Ok(())
    }

    /// Events buffered before this result. `checks` are rolls this result
    /// will print; a matching check event is not printed again.
    fn show_arrived(&mut self, checks: &[String]) {
        let mut pending: VecDeque<&str> = checks.iter().map(String::as_str).collect();
        let events = self.client.drain_events();
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
        for received in self.client.drain() {
            self.show_received(received);
        }
    }

    /// An event as `show_event` shows it, or an event that could not be read as its line
    /// on stderr.
    fn show_received(&mut self, received: Result<GmEvent, Error>) {
        match received {
            Ok(event) => self.show_event(&event),
            Err(err) => {
                eprintln!("{err}");
                let _ = io::stderr().flush();
            }
        }
    }

    fn show_event(&mut self, event: &GmEvent) {
        if self.raw {
            emit(&event_line(event));
            return;
        }
        let line = render_event(event);
        if matches!(event, GmEvent::Check(_)) && self.printed_checks.skip(&line) {
            return;
        }
        emit(&line);
    }

    /// Raw mode prints `line`, the refusal's result line, then the code on stderr.
    fn show_refusal(&self, refusal: &Refusal, line: &str) {
        if self.raw {
            emit(line);
        }
        eprintln!("{}", refusal_text(refusal, self.raw));
        let _ = io::stderr().flush();
    }
}

/// What a refusal prints on stderr: the code in raw mode, the sentence in text mode.
fn refusal_text(refusal: &Refusal, raw: bool) -> String {
    if raw {
        refusal.code.as_str().to_string()
    } else {
        refusal.to_string()
    }
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

/// The command a typed line names, or `None` when it names none.
fn parse_command(line: &str) -> Option<Command> {
    let (head, rest) = split_word(line);
    match head {
        "status" if rest.is_empty() => Some(Command::Status),
        "commit" if rest.is_empty() => Some(Command::Commit),
        "help" if rest.is_empty() => Some(Command::Help),
        "quit" if rest.is_empty() => Some(Command::Quit),
        "look" => {
            if rest.is_empty() || rest.split_whitespace().nth(1).is_some() {
                None
            } else {
                Some(Command::Look {
                    actor: rest.to_string(),
                })
            }
        }
        "say" => parse_say(rest),
        "queue" => parse_queue(rest),
        _ => None,
    }
}

fn parse_say(rest: &str) -> Option<Command> {
    if rest.is_empty() {
        return None;
    }
    if let Some(rest) = rest.strip_prefix('@') {
        let (place, text) = split_word(rest.trim());
        if place.is_empty() || text.is_empty() {
            return None;
        }
        return Some(Command::Say(SayArgs::Place {
            place: place.to_string(),
            text: text.to_string(),
        }));
    }
    if let Some(rest) = rest.strip_prefix('#') {
        let (actor, text) = split_word(rest.trim());
        if actor.is_empty() || text.is_empty() {
            return None;
        }
        return Some(Command::Say(SayArgs::Actor {
            actor: actor.to_string(),
            text: text.to_string(),
        }));
    }
    Some(Command::Say(SayArgs::Table {
        text: rest.to_string(),
    }))
}

fn parse_queue(rest: &str) -> Option<Command> {
    let mut parts = rest.split_whitespace();
    let actor = parts.next()?;
    let move_name = parts.next()?;
    let target = parts.next().map(str::to_string);
    if parts.next().is_some() {
        return None;
    }
    Some(Command::Queue {
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
    use crate::protocol::{Outcome, Roll};
    use crate::text::render_check;

    fn check_line(faces: &[u32], stat: i64, total: i64, target: i64, outcome: Outcome) -> String {
        render_check(&Roll {
            faces: faces.to_vec(),
            stat,
            total,
            target,
            outcome,
        })
    }

    #[test]
    fn a_commit_skips_its_own_check_events_once() {
        let roll = check_line(&[4, 2], 1, 7, 0, Outcome::Success);
        let mut printed = PrintedChecks::default();
        printed.replace(vec![roll.clone()]);
        assert!(printed.skip(&roll));
        // A later check with the same faces is a new roll and still prints.
        assert!(!printed.skip(&roll));
        assert!(!printed.skip(&roll));
    }

    #[test]
    fn checks_after_a_new_result_are_not_skipped() {
        let roll = check_line(&[3, 3], 0, 6, 0, Outcome::Success);
        let mut printed = PrintedChecks::default();
        printed.replace(vec![roll.clone()]);
        printed.clear();
        assert!(!printed.skip(&roll));
    }

    #[test]
    fn a_check_that_is_not_next_ends_the_skip() {
        let first = check_line(&[1, 1], 0, 2, 7, Outcome::Miss);
        let second = check_line(&[6, 6], 0, 12, 7, Outcome::Success);
        let mut printed = PrintedChecks::default();
        printed.replace(vec![first.clone(), second.clone()]);
        assert!(!printed.skip(&second));
        assert!(!printed.skip(&first));
    }

    #[test]
    fn a_commit_with_two_checks_skips_both_in_order() {
        let first = check_line(&[1, 2], 0, 3, 7, Outcome::Miss);
        let second = check_line(&[5, 6], 0, 11, 7, Outcome::Success);
        let mut printed = PrintedChecks::default();
        printed.replace(vec![first.clone(), second.clone()]);
        assert!(printed.skip(&first));
        assert!(printed.skip(&second));
        assert!(!printed.skip(&first));
    }

    fn command(line: &str) -> Command {
        parse_command(line).unwrap_or_else(|| panic!("not a command: {line}"))
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
        assert!(parse_command("").is_none());
        assert!(parse_command("   ").is_none());
        assert!(parse_command("dance").is_none());
        assert!(parse_command("status now").is_none());
        assert!(parse_command("look").is_none());
        assert!(parse_command("look a b").is_none());
        assert!(parse_command("say").is_none());
        assert!(parse_command("say @docks_shop").is_none());
        assert!(parse_command("say #mira").is_none());
        assert!(parse_command("queue mira").is_none());
        assert!(parse_command("queue mira travel docks_street extra").is_none());
    }
}
