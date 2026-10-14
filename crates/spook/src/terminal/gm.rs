use std::collections::VecDeque;
use std::io::{self, Write};
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use serde_json::{json, Value};
use spookery_kernel::canon::{Canon, CastKind};
use spookery_kernel::rules::rule::{TargetKind, When};

use crate::gm_connection::{self, Event, EventBody, GmConnection, Incoming, ToolCallResult};
use crate::protocol::PlayerMessage;

use super::render::{self, humanize, refusal};

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
      Commit the queue and tick.
  help
      Show these commands.
  quit
      Leave the seat.
An unknown command prints this list and sends nothing.";

pub fn run_gm_terminal(pack: &Path, addr: &str, raw: bool) -> Result<(), super::TerminalError> {
    let client = match GmConnection::connect(addr) {
        Ok(client) => client,
        Err(gm_connection::GmConnectionError::Io(err)) => {
            return Err(super::TerminalError::Io(err))
        }
        Err(gm_connection::GmConnectionError::Refused { code, raw: line }) => {
            exit_refused(&code, &line, raw)
        }
        Err(gm_connection::GmConnectionError::Bad(line)) => {
            eprintln!("{line}");
            let _ = io::stderr().flush();
            std::process::exit(1);
        }
        Err(gm_connection::GmConnectionError::Closed) => {
            eprintln!("seat closed");
            let _ = io::stderr().flush();
            std::process::exit(1);
        }
    };
    let canon = match spookery_kernel::canon::load_canon(pack) {
        Ok(canon) => canon,
        Err(errors) => {
            eprintln!("{}", spookery_kernel::first_reason(&errors));
            let _ = io::stderr().flush();
            std::process::exit(1);
        }
    };
    let mut session = Session {
        client,
        canon,
        raw,
        printed_checks: PrintedChecks::default(),
    };
    if raw && !session.client.opened_line().is_empty() {
        emit(session.client.opened_line());
    }
    emit(HELP);
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for line in io::stdin().lines() {
            match line {
                Ok(line) => {
                    if tx.send(Some(line)).is_err() {
                        return;
                    }
                }
                Err(_) => {
                    let _ = tx.send(None);
                    return;
                }
            }
        }
        let _ = tx.send(None);
    });
    loop {
        if let Some(incoming) = session.client.poll(Duration::from_millis(50)) {
            if !session.show_idle(incoming) {
                break;
            }
        }
        match rx.try_recv() {
            Ok(Some(line)) => {
                if !session.on_line(&line) {
                    break;
                }
            }
            Ok(None) | Err(mpsc::TryRecvError::Disconnected) => break,
            Err(mpsc::TryRecvError::Empty) => {}
        }
    }
    Ok(())
}

struct Session {
    client: GmConnection,
    canon: Canon,
    raw: bool,
    printed_checks: PrintedChecks,
}

/// Check lines `commit` already printed, in the order their check events
/// follow the result on the socket. Each matching event is skipped once, so a
/// later check still prints.
#[derive(Debug, Default)]
struct PrintedChecks(VecDeque<String>);

impl PrintedChecks {
    /// The check lines one commit result printed. Replaces any older lines.
    fn printed(&mut self, lines: Vec<String>) {
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
    fn on_line(&mut self, line: &str) -> bool {
        match parse_command(line) {
            Parsed::Empty => true,
            Parsed::Unknown => {
                emit(HELP);
                true
            }
            Parsed::Command(command) => self.on_command(command),
        }
    }

    fn on_command(&mut self, command: Command) -> bool {
        match command {
            Command::Help => emit(HELP),
            Command::Quit => return false,
            Command::Status => self.on_status(),
            Command::Look { actor } => self.on_look(&actor),
            Command::Say(address) => self.on_say(address),
            Command::Queue {
                actor,
                move_name,
                target,
            } => self.on_queue(&actor, &move_name, target.as_deref()),
            Command::Commit => self.on_commit(),
        }
        true
    }

    fn on_status(&mut self) {
        let Some(outcome) = self.call_tool("session_status", json!({})) else {
            return;
        };
        self.after_result(&outcome, |_session, outcome| {
            emit(&render_status(&outcome.body));
        });
    }

    fn on_look(&mut self, actor: &str) {
        let Some((id, kind)) = self.resolve(actor) else {
            eprintln!("{}", refusal("UNKNOWN_KEY"));
            let _ = io::stderr().flush();
            return;
        };
        let as_viewer = format!("{kind}:{id}");
        let Some(outcome) = self.call_tool("perceive", json!({ "as": as_viewer })) else {
            return;
        };
        self.after_result(&outcome, |_session, outcome| {
            match serde_json::from_value::<PlayerMessage>(outcome.body.clone()) {
                Ok(message) => emit(&render::render(&message)),
                Err(_) => {
                    eprintln!("The reply could not be read.");
                    let _ = io::stderr().flush();
                }
            }
        });
    }

    fn on_say(&mut self, address: Address) {
        let arguments = say_arguments(&address);
        let Some(outcome) = self.call_tool("say", arguments) else {
            return;
        };
        self.after_result(&outcome, |_session, _outcome| {});
    }

    fn on_queue(&mut self, actor: &str, move_name: &str, target: Option<&str>) {
        let Some((id, kind)) = self.resolve(actor) else {
            eprintln!("{}", refusal("UNKNOWN_KEY"));
            let _ = io::stderr().flush();
            return;
        };
        let target = target_value(&self.canon, move_name, target);
        let shown = format!("{} queued: {move_name}", actor_name(&id));
        let arguments = json!({
            "actor_id": id,
            "actor_kind": kind,
            "source": "player",
            "move": move_name,
            "target": target,
        });
        let Some(outcome) = self.call_tool("intent", arguments) else {
            return;
        };
        self.after_result(&outcome, move |_session, _outcome| {
            emit(&shown);
        });
    }

    fn on_commit(&mut self) {
        let Some(outcome) = self.call_tool("commit_turn", json!({ "include_tick": true })) else {
            return;
        };
        self.show_early();
        if outcome.refused {
            self.show_tool_refusal(&outcome);
            self.show_ready();
            return;
        }
        if self.raw {
            emit(&outcome.raw);
            self.show_ready();
            return;
        }
        emit(&render_diff(&outcome.body));
        let checks = render_checks(&outcome.body);
        for line in &checks {
            emit(line);
        }
        // The seat writes this commit's check events right after its result,
        // so each one is skipped once and later checks still print.
        self.printed_checks.printed(checks);
        self.show_ready();
    }

    fn call_tool(&mut self, name: &str, arguments: Value) -> Option<ToolCallResult> {
        match self.client.call(name, arguments) {
            Ok(outcome) => Some(outcome),
            Err(gm_connection::GmConnectionError::Io(err)) => {
                eprintln!("{err}");
                let _ = io::stderr().flush();
                std::process::exit(1);
            }
            Err(gm_connection::GmConnectionError::Closed) => None,
            Err(gm_connection::GmConnectionError::Refused { code, raw }) => {
                self.show_refusal(&code, &raw);
                None
            }
            Err(gm_connection::GmConnectionError::Bad(line)) => {
                if self.raw {
                    emit(&line);
                } else {
                    eprintln!("{line}");
                    let _ = io::stderr().flush();
                }
                None
            }
        }
    }

    /// Text mode prints `render`. Raw mode already printed the result line.
    fn after_result(
        &mut self,
        outcome: &ToolCallResult,
        render: impl FnOnce(&mut Self, &ToolCallResult),
    ) {
        self.show_early();
        if outcome.refused {
            self.show_tool_refusal(outcome);
            self.show_ready();
            return;
        }
        if self.raw {
            emit(&outcome.raw);
        } else {
            render(self, outcome);
        }
        self.show_ready();
    }

    fn show_early(&mut self) {
        let events = self.client.take_events();
        for event in &events {
            self.show_event(event);
        }
        // Every check event of the previous commit came before this result.
        self.printed_checks.clear();
    }

    fn show_ready(&mut self) {
        let ready = self.client.drain_ready();
        for incoming in ready {
            let _ = self.show_idle(incoming);
        }
    }

    fn show_idle(&mut self, incoming: Incoming) -> bool {
        match incoming {
            Incoming::Event(event) => self.show_event(&event),
            Incoming::Result { raw, .. } => {
                if self.raw {
                    emit(&raw);
                }
            }
            Incoming::Bad(line) => {
                eprintln!("{line}");
                let _ = io::stderr().flush();
            }
            Incoming::Refused { code, raw } => self.show_refusal(&code, &raw),
            Incoming::Eof => return false,
        }
        true
    }

    fn show_event(&mut self, event: &Event) {
        if self.raw {
            emit(&event.raw);
            return;
        }
        let line = render_event(&event.body);
        if matches!(event.body, EventBody::Check { .. }) && self.printed_checks.skip(&line) {
            return;
        }
        emit(&line);
    }

    fn show_tool_refusal(&self, outcome: &ToolCallResult) {
        let code = outcome
            .body
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or("");
        self.show_refusal(code, &outcome.raw);
    }

    fn show_refusal(&self, code: &str, raw: &str) {
        if self.raw {
            emit(raw);
            eprintln!("{code}");
        } else {
            eprintln!("{}", refusal(code));
        }
        let _ = io::stderr().flush();
    }

    fn resolve(&mut self, token: &str) -> Option<(String, String)> {
        if let Some(id) = prefixed(token, "player:") {
            return Some((id, "player".to_string()));
        }
        if let Some(id) = prefixed(token, "npc:") {
            return Some((id, "npc".to_string()));
        }
        let id = token.replace(' ', "_");
        if let Some(cast) = self.canon.cast.get(&id) {
            let kind = match cast.kind {
                CastKind::Npc => "npc",
                CastKind::PcTemplate => "player",
            };
            return Some((id, kind.to_string()));
        }
        if self.present_ids().iter().any(|actor| actor == &id) {
            return Some((id, "player".to_string()));
        }
        None
    }

    fn present_ids(&mut self) -> Vec<String> {
        let Some(outcome) = self.call_tool("session_status", json!({})) else {
            return Vec::new();
        };
        self.show_early();
        if outcome.refused {
            self.show_tool_refusal(&outcome);
            self.show_ready();
            return Vec::new();
        }
        if self.raw {
            emit(&outcome.raw);
        }
        let ids = seated_ids(&outcome.body);
        self.show_ready();
        ids
    }
}

fn prefixed(token: &str, prefix: &str) -> Option<String> {
    let id = token.strip_prefix(prefix)?;
    if id.is_empty() || id.chars().any(char::is_whitespace) {
        return None;
    }
    Some(id.to_string())
}

fn say_arguments(address: &Address) -> Value {
    match address {
        Address::Table { text } => json!({ "address": "table", "text": text }),
        Address::Place { place, text } => {
            json!({ "address": "place", "place": place, "text": text })
        }
        Address::Actor { actor, text } => {
            json!({ "address": "actor", "actor": bare_id(actor), "text": text })
        }
    }
}

fn bare_id(token: &str) -> &str {
    token
        .strip_prefix("player:")
        .or_else(|| token.strip_prefix("npc:"))
        .unwrap_or(token)
}

fn target_value(canon: &Canon, move_name: &str, target: Option<&str>) -> Value {
    let Some(target) = target else {
        return json!({});
    };
    let kind = canon.rules.iter().find_map(|rule| match &rule.when {
        When::Verb { verb, target } if verb == move_name => Some(*target),
        When::Habit if rule.local_id == move_name => Some(TargetKind::None),
        _ => None,
    });
    match kind {
        Some(TargetKind::Actor) | Some(TargetKind::Item) => json!({ "actor": target }),
        _ => json!({ "place": target }),
    }
}

fn exit_refused(code: &str, raw: &str, raw_mode: bool) -> ! {
    if raw_mode {
        emit(raw);
        eprintln!("{code}");
    } else {
        eprintln!("{}", refusal(code));
    }
    let _ = io::stderr().flush();
    std::process::exit(1);
}

fn emit(line: &str) {
    println!("{line}");
    let _ = io::stdout().flush();
}

fn seated_ids(body: &Value) -> Vec<String> {
    body.get("presence")
        .and_then(|presence| presence.get("actors_present"))
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| row.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn queue_rows(body: &Value) -> Vec<(String, String)> {
    body.get("queued")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| {
                    Some((
                        row.get("actor_id")?.as_str()?.to_string(),
                        row.get("move")?.as_str()?.to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn render_status(body: &Value) -> String {
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

fn render_diff(body: &Value) -> String {
    let rows = body
        .get("diff_summary")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| row.as_str().map(str::to_string))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if rows.is_empty() {
        "Nothing changed.".to_string()
    } else {
        format!("Changed:\n{}", rows.join("\n"))
    }
}

fn render_checks(body: &Value) -> Vec<String> {
    let Some(checks) = body.get("checks").and_then(Value::as_array) else {
        return Vec::new();
    };
    checks.iter().filter_map(check_line).collect()
}

fn check_line(item: &Value) -> Option<String> {
    let faces = item.get("faces")?.as_array()?;
    let faces = faces
        .iter()
        .map(Value::as_u64)
        .collect::<Option<Vec<_>>>()?;
    Some(render_check(
        &faces,
        item.get("stat")?.as_i64()?,
        item.get("total")?.as_i64()?,
        item.get("target")?.as_i64()?,
        check_word(item.get("outcome")?)?,
    ))
}

fn check_word(value: &Value) -> Option<&'static str> {
    match value.as_str()? {
        "success" => Some("success"),
        "partial" => Some("partial"),
        "miss" => Some("miss"),
        _ => None,
    }
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
        EventBody::Say { from, text, .. } => format!("{}: {text}", actor_name(from)),
        EventBody::Queued {
            actor_id,
            move_name,
            ..
        } => format!("{} queued: {move_name}", actor_name(actor_id)),
        EventBody::Check {
            faces,
            stat,
            total,
            target,
            outcome,
        } => render_check(faces, *stat, *total, *target, outcome.as_str()),
    }
}

pub fn render_check(faces: &[u64], stat: i64, total: i64, target: i64, outcome: &str) -> String {
    let dice = faces
        .iter()
        .map(u64::to_string)
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
    Say(Address),
    Queue {
        actor: String,
        move_name: String,
        target: Option<String>,
    },
    Commit,
    Help,
    Quit,
}

#[derive(Debug)]
pub enum Address {
    Table { text: String },
    Place { place: String, text: String },
    Actor { actor: String, text: String },
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
        return Parsed::Command(Command::Say(Address::Place {
            place: place.to_string(),
            text: text.to_string(),
        }));
    }
    if let Some(rest) = rest.strip_prefix('#') {
        let (actor, text) = split_word(rest.trim());
        if actor.is_empty() || text.is_empty() {
            return Parsed::Unknown;
        }
        return Parsed::Command(Command::Say(Address::Actor {
            actor: actor.to_string(),
            text: text.to_string(),
        }));
    }
    Parsed::Command(Command::Say(Address::Table {
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
    use crate::protocol::Outcome;

    #[test]
    fn a_commit_skips_its_own_check_events_once() {
        let roll = render_check(&[4, 2], 1, 7, 0, "success");
        let mut printed = PrintedChecks::default();
        printed.printed(vec![roll.clone()]);
        assert!(printed.skip(&roll));
        // A later check with the same faces is a new roll and still prints.
        assert!(!printed.skip(&roll));
        assert!(!printed.skip(&roll));
    }

    #[test]
    fn checks_after_a_new_result_are_not_skipped() {
        let roll = render_check(&[3, 3], 0, 6, 0, "success");
        let mut printed = PrintedChecks::default();
        printed.printed(vec![roll.clone()]);
        printed.clear();
        assert!(!printed.skip(&roll));
    }

    #[test]
    fn a_check_that_is_not_next_ends_the_skip() {
        let first = render_check(&[1, 1], 0, 2, 7, "miss");
        let second = render_check(&[6, 6], 0, 12, 7, "success");
        let mut printed = PrintedChecks::default();
        printed.printed(vec![first.clone(), second.clone()]);
        assert!(!printed.skip(&second));
        assert!(!printed.skip(&first));
    }

    #[test]
    fn a_commit_with_two_checks_skips_both_in_order() {
        let first = render_check(&[1, 2], 0, 3, 7, "miss");
        let second = render_check(&[5, 6], 0, 11, 7, "success");
        let mut printed = PrintedChecks::default();
        printed.printed(vec![first.clone(), second.clone()]);
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
        match command("look player:pc_rowan") {
            Command::Look { actor } => assert_eq!(actor, "player:pc_rowan"),
            other => panic!("{other:?}"),
        }
        match command("say good evening") {
            Command::Say(Address::Table { text }) => assert_eq!(text, "good evening"),
            other => panic!("{other:?}"),
        }
        match command("say @docks_shop the lamp is lit") {
            Command::Say(Address::Place { place, text }) => {
                assert_eq!(place, "docks_shop");
                assert_eq!(text, "the lamp is lit");
            }
            other => panic!("{other:?}"),
        }
        match command("say #mira come in") {
            Command::Say(Address::Actor { actor, text }) => {
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
        let say = render_event(&EventBody::Say {
            from: "player:pc_rowan".to_string(),
            seq: 1,
            address: "table".to_string(),
            place: None,
            actor: None,
            text: "hello".to_string(),
        });
        assert_eq!(say, "Pc rowan: hello");
        assert!(!say.contains('{'));

        let gm = render_event(&EventBody::Say {
            from: "gm".to_string(),
            seq: 2,
            address: "table".to_string(),
            place: None,
            actor: None,
            text: "good evening".to_string(),
        });
        assert_eq!(gm, "GM: good evening");

        let npc = render_event(&EventBody::Say {
            from: "npc:mira".to_string(),
            seq: 3,
            address: "table".to_string(),
            place: None,
            actor: None,
            text: "oil".to_string(),
        });
        assert_eq!(npc, "Mira: oil");

        let queued = render_event(&EventBody::Queued {
            id: "id-1".to_string(),
            actor_id: "pc_wren".to_string(),
            move_name: "wait".to_string(),
        });
        assert_eq!(queued, "Pc wren queued: wait");

        let success = render_event(&EventBody::Check {
            faces: vec![4, 2],
            stat: 1,
            total: 7,
            target: 0,
            outcome: Outcome::Success,
        });
        assert_eq!(success, "Roll: 4 + 2, +1 = 7 vs 0, success");

        let partial = render_event(&EventBody::Check {
            faces: vec![3, 3],
            stat: 1,
            total: 7,
            target: 10,
            outcome: Outcome::Partial,
        });
        assert_eq!(partial, "Roll: 3 + 3, +1 = 7 vs 10, partial");

        let failure = render_event(&EventBody::Check {
            faces: vec![1],
            stat: -2,
            total: -1,
            target: 3,
            outcome: Outcome::Miss,
        });
        assert_eq!(failure, "Roll: 1, -2 = -1 vs 3, miss");
        assert!(!failure.contains('{'));
    }

    #[test]
    fn a_status_names_the_seated_players_and_the_queue() {
        let body = json!({
            "presence": { "humans": 1, "actors_present": ["pc_rowan"] },
            "queued": [{ "id": "q", "actor_id": "pc_rowan", "move": "wait" }]
        });
        assert_eq!(
            render_status(&body),
            "Seated: Pc rowan\nQueue:\nPc rowan: wait"
        );
        let empty = json!({
            "presence": { "humans": 0, "actors_present": [] },
            "queued": []
        });
        assert_eq!(render_status(&empty), "Seated: nobody\nQueue: empty");
    }
}
