//! The human GM: commands in, tool calls out.

use std::collections::VecDeque;
use std::io::{self, Write};
use std::net::SocketAddr;

use serde_json::{json, Value};

use crate::gm_connection::{self, Connection, Event, EventBody, Incoming, ToolResult};
use crate::protocol::{CommitBody, PlayerFrame, SayArgs, TableStatus};

use super::frame_text::{self, humanize};
use super::refusal_line;

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

pub async fn run_gm(
    addr: SocketAddr,
    door: Option<SocketAddr>,
    raw: bool,
) -> Result<(), super::Error> {
    let connected = match door {
        Some(door) => Connection::connect_door(door).await,
        None => Connection::connect_default_door(addr).await,
    };
    let client = match connected {
        Ok(client) => client,
        Err(gm_connection::Error::Io(err)) => return Err(super::Error::Io(err)),
        Err(gm_connection::Error::Refused { code, raw: line }) => {
            return Err(super::Error::Message(echo_refusal(&code, &line, raw)));
        }
        Err(gm_connection::Error::Invalid(line)) => return Err(super::Error::Message(line)),
    };
    let mut session = Session {
        connection: client,
        is_raw: raw,
        printed_checks: PrintedChecks::default(),
    };
    if raw && !session.connection.initialize_line().is_empty() {
        emit(session.connection.initialize_line());
    }
    emit(HELP);
    let mut input = super::stdin_lines();
    let result = async {
        loop {
            tokio::select! {
                incoming = session.connection.next_incoming() => {
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
    session.connection.close().await;
    result
}

struct Session {
    connection: Connection,
    is_raw: bool,
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
    async fn on_line(&mut self, line: &str) -> Result<bool, super::Error> {
        match parse_command(line) {
            Parsed::Empty => Ok(true),
            Parsed::Unknown => {
                emit(HELP);
                Ok(true)
            }
            Parsed::Command(command) => self.on_command(command).await,
        }
    }

    async fn on_command(&mut self, command: Command) -> Result<bool, super::Error> {
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

    async fn on_status(&mut self) -> Result<(), super::Error> {
        let Some(outcome) = self.call_tool("table_status", json!({})).await? else {
            return Ok(());
        };
        self.show_result(&outcome, |_session, outcome| {
            match read_status(&outcome.body) {
                Some(status) => emit(&render_status(&status)),
                None => report_unreadable(),
            }
        });
        Ok(())
    }

    async fn on_look(&mut self, actor: &str) -> Result<(), super::Error> {
        let Some(outcome) = self.call_tool("perceive", json!({ "as": actor })).await? else {
            return Ok(());
        };
        self.show_result(&outcome, |_session, outcome| {
            match serde_json::from_value::<PlayerFrame>(outcome.body.clone()) {
                Ok(message) => emit(&frame_text::render(&message)),
                Err(_) => {
                    eprintln!("The reply could not be read.");
                    let _ = io::stderr().flush();
                }
            }
        });
        Ok(())
    }

    async fn on_say(&mut self, address: SayArgs) -> Result<(), super::Error> {
        let arguments = address.to_value();
        let Some(outcome) = self.call_tool("say", arguments).await? else {
            return Ok(());
        };
        self.show_result(&outcome, |_session, _outcome| {});
        Ok(())
    }

    async fn on_queue(
        &mut self,
        actor: &str,
        move_name: &str,
        target: Option<&str>,
    ) -> Result<(), super::Error> {
        let shown = format!("{} queued: {move_name}", actor_name(actor));
        let mut arguments = json!({
            "actor_id": actor,
            "move": move_name,
        });
        if let Some(t) = target {
            arguments["target"] = json!(t);
        }
        let Some(outcome) = self.call_tool("queue_intent", arguments).await? else {
            return Ok(());
        };
        self.show_result(&outcome, move |_session, _outcome| {
            emit(&shown);
        });
        Ok(())
    }

    async fn on_commit(&mut self) -> Result<(), super::Error> {
        let Some(outcome) = self
            .call_tool("commit_round", json!({ "include_round": true }))
            .await?
        else {
            return Ok(());
        };
        let reply = if outcome.refused {
            None
        } else {
            read_commit(&outcome.body)
        };
        // This commit's check events can arrive before or after the tool
        // result. The result text prints each roll. A matching event on
        // either side is shown once. Raw mode and a failed read show none.
        let checks = reply.as_ref().filter(|_| !self.is_raw).map(render_checks);
        self.show_arrived(checks.as_deref().unwrap_or(&[]));
        if outcome.refused {
            self.show_tool_refusal(&outcome);
        } else if let Some(reply) = reply {
            if self.is_raw {
                emit(&outcome.raw);
            } else if let Some(checks) = checks {
                emit(&render_diff(&reply));
                for line in &checks {
                    emit(line);
                }
                for line in &render_dice(&reply) {
                    emit(line);
                }
                self.printed_checks.replace(checks);
            }
        } else {
            report_unreadable();
        }
        self.show_ready();
        Ok(())
    }

    async fn call_tool(
        &mut self,
        name: &str,
        arguments: Value,
    ) -> Result<Option<ToolResult>, super::Error> {
        match self.connection.call(name, arguments).await {
            Ok(outcome) => Ok(Some(outcome)),
            Err(gm_connection::Error::Io(err)) => Err(super::Error::Io(err)),
            Err(gm_connection::Error::Refused { code, raw }) => {
                self.show_refusal(&code, &raw);
                Ok(None)
            }
            Err(gm_connection::Error::Invalid(line)) => {
                if self.is_raw {
                    emit(&line);
                } else {
                    eprintln!("{line}");
                    let _ = io::stderr().flush();
                }
                Ok(None)
            }
        }
    }

    /// Text mode prints `frame_text`. Raw mode already printed the result line.
    fn show_result(&mut self, outcome: &ToolResult, render: impl FnOnce(&mut Self, &ToolResult)) {
        self.show_early();
        if outcome.refused {
            self.show_tool_refusal(outcome);
            self.show_ready();
            return;
        }
        if self.is_raw {
            emit(&outcome.raw);
        } else {
            render(self, outcome);
        }
        self.show_ready();
    }

    fn show_early(&mut self) {
        self.show_arrived(&[]);
    }

    /// Events buffered before this result. `checks` are rolls this result
    /// will print; a matching check event is not printed again.
    fn show_arrived(&mut self, checks: &[String]) {
        let mut pending: VecDeque<&str> = checks.iter().map(String::as_str).collect();
        let events = self.connection.take_events();
        for event in &events {
            if let EventBody::Check(_) = event.body {
                let line = render_event(&event.body);
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
        let ready = self.connection.drain_ready();
        for incoming in ready {
            let _ = self.show_incoming(incoming);
        }
    }

    fn show_incoming(&mut self, incoming: Incoming) -> bool {
        match incoming {
            Incoming::Event(event) => self.show_event(&event),
            Incoming::Result { raw, .. } => {
                if self.is_raw {
                    emit(&raw);
                }
            }
            Incoming::Invalid(line) => {
                eprintln!("{line}");
                let _ = io::stderr().flush();
            }
            Incoming::Refused { code, raw } => self.show_refusal(&code, &raw),
            Incoming::Eof => return false,
        }
        true
    }

    fn show_event(&mut self, event: &Event) {
        if self.is_raw {
            emit(&event.raw);
            return;
        }
        let line = render_event(&event.body);
        if matches!(event.body, EventBody::Check(_)) && self.printed_checks.skip(&line) {
            return;
        }
        emit(&line);
    }

    fn show_tool_refusal(&self, outcome: &ToolResult) {
        let code = outcome
            .body
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or("");
        self.show_refusal(code, &outcome.raw);
    }

    fn show_refusal(&self, code: &str, raw: &str) {
        eprintln!("{}", echo_refusal(code, raw, self.is_raw));
        let _ = io::stderr().flush();
    }
}

/// Raw mode echoes the frame and names the code. Text mode names the refusal.
fn echo_refusal(code: &str, raw: &str, raw_mode: bool) -> String {
    if raw_mode {
        emit(raw);
        code.to_string()
    } else {
        refusal_line(code)
    }
}

fn emit(line: &str) {
    println!("{line}");
    let _ = io::stdout().flush();
}

fn read_status(body: &Value) -> Option<TableStatus> {
    serde_json::from_value(body.clone()).ok()
}

fn read_commit(body: &Value) -> Option<CommitBody> {
    serde_json::from_value(body.clone()).ok()
}

fn report_unreadable() {
    eprintln!("The reply could not be read.");
    let _ = io::stderr().flush();
}

fn seated_ids(body: &TableStatus) -> Vec<String> {
    body.presence.actors_present.clone()
}

fn queue_rows(body: &TableStatus) -> Vec<(String, String)> {
    body.queued
        .iter()
        .map(|row| (row.actor_id.clone(), row.move_name.clone()))
        .collect()
}

fn render_status(body: &TableStatus) -> String {
    let names = seated_ids(body);
    let seated = if names.is_empty() {
        "Seated: nobody".to_string()
    } else {
        let names = names
            .iter()
            .map(|id| actor_name(id))
            .collect::<Vec<_>>()
            .join(", ");
        format!("Seated: {names}")
    };
    let queue = queue_rows(body);
    let queued = if queue.is_empty() {
        "Queue: empty".to_string()
    } else {
        let lines = queue
            .iter()
            .map(|(id, move_name)| format!("{}: {move_name}", actor_name(id)))
            .collect::<Vec<_>>()
            .join("\n");
        format!("Queue:\n{lines}")
    };
    format!("{seated}\n{queued}")
}

fn render_diff(body: &CommitBody) -> String {
    if body.changed_keys.is_empty() {
        "Nothing changed.".to_string()
    } else {
        format!("Changed:\n{}", body.changed_keys.join("\n"))
    }
}

fn render_dice(body: &CommitBody) -> Vec<String> {
    body.dice
        .iter()
        .map(|line| format!("dice {line}"))
        .collect()
}

fn render_checks(body: &CommitBody) -> Vec<String> {
    body.rolls
        .iter()
        .map(|roll| {
            render_check(
                &roll.faces,
                roll.stat,
                roll.total,
                roll.target,
                roll.outcome.as_str(),
            )
        })
        .collect()
}

pub fn actor_name(id: &str) -> String {
    if id == "gm" {
        "GM".to_string()
    } else {
        humanize(id)
    }
}

pub fn render_event(body: &EventBody) -> String {
    match body {
        EventBody::Say(say) => format!("{}: {}", actor_name(&say.from), say.text),
        EventBody::Queued(row) => {
            format!("{} queued: {}", actor_name(&row.actor_id), row.move_name)
        }
        EventBody::Check(roll) => render_check(
            &roll.faces,
            roll.stat,
            roll.total,
            roll.target,
            roll.outcome.as_str(),
        ),
    }
}

pub fn render_check(faces: &[u32], stat: i64, total: i64, target: i64, outcome: &str) -> String {
    let dice = faces
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(" + ");
    let sign = if stat >= 0 {
        format!("+{stat}")
    } else {
        stat.to_string()
    };
    if dice.is_empty() {
        format!("Roll: {sign} = {total} vs {target}, {outcome}")
    } else {
        format!("Roll: {dice}, {sign} = {total} vs {target}, {outcome}")
    }
}

#[derive(Debug)]
pub enum Command {
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

pub enum Parsed {
    Empty,
    Command(Command),
    Unknown,
}

pub fn parse_command(line: &str) -> Parsed {
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
    use crate::protocol::{Outcome, QueuedMove, Roll, SayEvent, StatusPause, StatusPresence};

    fn say(from: &str, seq: u64, text: &str) -> EventBody {
        EventBody::Say(SayEvent {
            seq,
            from: from.to_string(),
            address: "table".to_string(),
            place: None,
            actor: None,
            recipients: Vec::new(),
            text: text.to_string(),
        })
    }

    fn roll(faces: Vec<u32>, stat: i64, total: i64, target: i64, outcome: Outcome) -> EventBody {
        EventBody::Check(Roll {
            faces,
            stat,
            total,
            target,
            outcome,
        })
    }

    fn status(present: Vec<&str>, queued: Vec<QueuedMove>) -> TableStatus {
        TableStatus {
            presence: StatusPresence {
                players: u32::try_from(present.len()).unwrap_or(0),
                actors_present: present.into_iter().map(str::to_string).collect(),
            },
            pause: StatusPause {
                presence_clocks: "running".to_string(),
            },
            world_time: 0,
            round_index: 0,
            state_rev: 0,
            pack_rev: String::new(),
            last_round_at: String::new(),
            queued,
            last_seq: 0,
            events: None,
            pack: None,
        }
    }

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

    #[test]
    fn each_event_renders_as_text() {
        let player = render_event(&say("player:pc_rowan", 1, "hello"));
        assert_eq!(player, "Pc rowan: hello");
        assert!(!player.contains('{'));

        let gm = render_event(&say("gm", 2, "good evening"));
        assert_eq!(gm, "GM: good evening");

        let npc = render_event(&say("npc:mira", 3, "oil"));
        assert_eq!(npc, "Mira: oil");

        let queued = render_event(&EventBody::Queued(QueuedMove {
            id: "id-1".to_string(),
            actor_id: "pc_wren".to_string(),
            move_name: "wait".to_string(),
        }));
        assert_eq!(queued, "Pc wren queued: wait");

        let success = render_event(&roll(vec![4, 2], 1, 7, 0, Outcome::Success));
        assert_eq!(success, "Roll: 4 + 2, +1 = 7 vs 0, success");

        let partial = render_event(&roll(vec![3, 3], 1, 7, 10, Outcome::Partial));
        assert_eq!(partial, "Roll: 3 + 3, +1 = 7 vs 10, partial");

        let failure = render_event(&roll(vec![1], -2, -1, 3, Outcome::Miss));
        assert_eq!(failure, "Roll: 1, -2 = -1 vs 3, miss");
        assert!(!failure.contains('{'));
    }

    #[test]
    fn a_status_names_the_seated_players_and_the_queue() {
        let body = status(
            vec!["pc_rowan"],
            vec![QueuedMove {
                id: "q".to_string(),
                actor_id: "pc_rowan".to_string(),
                move_name: "wait".to_string(),
            }],
        );
        assert_eq!(
            render_status(&body),
            "Seated: Pc rowan\nQueue:\nPc rowan: wait"
        );
        assert_eq!(
            render_status(&status(Vec::new(), Vec::new())),
            "Seated: nobody\nQueue: empty"
        );
    }
}
