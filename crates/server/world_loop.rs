//! The table loop: seat players, read lines, and answer the GM.

use std::io::Write;

use spookery_engine::{Address, GateRefusal, Presence, Reason, RecipientLine, RoundOutput, World};

use crate::protocol::Missing;

use super::frames;
use super::idle_clock::{self, wall_now};
use super::outbound::{CancelTx, OutTx};

/// The host's cap. A pack may define more PCs than this.
const MAX_PLAYER_SEATS: usize = 3;

/// Commit a queued round, then fast-forward while every seated PC is busy.
/// An empty queue only fast-forwards.
pub fn commit_then_fast_forward(
    world: &mut World,
    presence: &Presence,
) -> Result<Vec<RoundOutput>, Reason> {
    let wall = wall_now();
    world.commit_then_fast_forward(presence, &wall)
}

pub(super) fn run(
    mut world: World,
    rx: std::sync::mpsc::Receiver<Request>,
    restart: &tokio::sync::watch::Sender<u64>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<(), super::listener::Error> {
    let mut seats = Seats::default();
    loop {
        let msg = rx
            .recv()
            .map_err(|_| super::listener::Error::Reason(Reason::NotInSpec))?;
        if matches!(msg, Request::Stop) {
            shutdown(&world, &mut seats, out);
            return Ok(());
        }
        let handled = handle(&mut world, &mut seats, restart, out, msg);
        if let Some(warning) = world.take_warning() {
            let _ = writeln!(err, "{warning}");
            let _ = err.flush();
        }
        handled.map_err(super::listener::Error::Reason)?;
    }
}

fn write_digest(out: &mut dyn Write, world: &World, outputs: &[RoundOutput]) {
    let text = world.digest(outputs);
    if !text.is_empty() {
        let _ = out.write_all(text.as_bytes());
        let _ = out.flush();
    }
}

/// The one owner of who is seated. Rounds and readers take `presence()`.
#[derive(Default)]
struct Seats {
    players: Vec<PlayerSeat>,
    gm: Option<GmSeat>,
}

impl Seats {
    /// The seated PCs, sorted, so a round records the same `actors_present` bytes.
    fn presence(&self) -> Presence {
        let mut actors_present: Vec<String> = self
            .players
            .iter()
            .map(|player| player.actor_id.clone())
            .collect();
        actors_present.sort();
        Presence {
            players: u32::try_from(actors_present.len()).unwrap_or(u32::MAX),
            actors_present,
        }
    }

    fn player(&self, conn: u64) -> Option<&PlayerSeat> {
        self.players.iter().find(|player| player.conn == conn)
    }

    fn take_player(&mut self, conn: u64) -> Option<PlayerSeat> {
        let index = self.players.iter().position(|player| player.conn == conn)?;
        Some(self.players.remove(index))
    }
}

struct PlayerSeat {
    conn: u64,
    actor_id: String,
    out: OutTx,
    cancel: CancelTx,
}

impl Drop for PlayerSeat {
    fn drop(&mut self) {
        let _ = self.cancel.send(true);
    }
}

struct GmSeat {
    conn: u64,
    out: OutTx,
    cancel: CancelTx,
}

impl Drop for GmSeat {
    fn drop(&mut self) {
        let _ = self.cancel.send(true);
    }
}

pub(super) enum Request {
    Hello {
        conn: u64,
        line: String,
        outbound: OutTx,
        cancel: CancelTx,
        reply: tokio::sync::oneshot::Sender<HelloReply>,
    },
    Seated {
        conn: u64,
        line: String,
        reply: tokio::sync::oneshot::Sender<()>,
    },
    Hangup {
        conn: u64,
        ack: tokio::sync::oneshot::Sender<()>,
    },
    Tool {
        conn: u64,
        name: String,
        arguments: Option<serde_json::Map<String, serde_json::Value>>,
        reply: tokio::sync::oneshot::Sender<serde_json::Value>,
    },
    /// The HTTP session takes the one GM seat. The game port does not.
    SeatGm {
        conn: u64,
        out: OutTx,
        cancel: CancelTx,
        reply: tokio::sync::oneshot::Sender<Result<(), Reason>>,
    },
    Wake,
    Stop,
}

pub(super) enum HelloReply {
    Refused(String),
    Line,
}

fn handle(
    world: &mut World,
    seats: &mut Seats,
    restart: &tokio::sync::watch::Sender<u64>,
    out: &mut dyn Write,
    msg: Request,
) -> Result<(), Reason> {
    match msg {
        Request::Hello {
            conn,
            line,
            outbound: out_tx,
            cancel,
            reply,
        } => {
            let answer = on_hello(world, seats, conn, &line, out_tx, cancel, out);
            let _ = reply.send(answer);
        }
        Request::SeatGm {
            conn,
            out: out_tx,
            cancel,
            reply,
        } => match seat_gm(seats, conn, out_tx, cancel) {
            Ok(()) => {
                if reply.send(Ok(())).is_err() {
                    seats.gm = None;
                }
            }
            Err(code) => {
                let _ = reply.send(Err(code));
            }
        },
        Request::Seated { conn, line, reply } => {
            on_player(world, seats, restart, conn, &line, out);
            let _ = reply.send(());
        }
        Request::Hangup { conn, ack } => on_hangup(world, seats, conn, ack, out),
        Request::Tool {
            conn,
            name,
            arguments,
            reply,
        } => {
            if seats.gm.as_ref().is_none_or(|gm| gm.conn != conn) {
                let _ = reply.send(crate::protocol::code_result(Reason::NotInSpec.as_str()));
                return Ok(());
            }
            let presence = seats.presence();
            let mut done = super::gm_tools::call_gm_tool(world, &presence, &name, arguments);
            if let Some(code) = &done.refused {
                let _ = writeln!(out, "refused {name} {}", code.as_str());
                let _ = out.flush();
            }
            let outputs = std::mem::take(&mut done.outputs);
            let reports: Vec<_> = outputs.iter().flat_map(|o| o.reports.clone()).collect();
            if !outputs.is_empty() {
                refresh_players(world, seats, &outputs, out);
                write_digest(out, world, &outputs);
                restart_idle_timer(restart);
            }
            if let Some(tape) = &done.tape_line {
                let _ = writeln!(out, "{tape}");
                let _ = out.flush();
            }
            if let Some((recipients, line)) = &done.player_line {
                send_recipients(world, seats, recipients, line.clone(), out);
            }
            let mut after = done.gm_events;
            after.extend(frames::check_events(&reports));
            for frame in after {
                send_gm(seats, frame);
            }
            // Free the seat before the client can read this result.
            if done.release_seat {
                seats.gm = None;
            }
            let _ = reply.send(done.value);
        }
        Request::Wake => {
            if idle_clock::catch_up_if_due(world, !seats.players.is_empty(), seats.gm.is_some())? {
                restart_idle_timer(restart);
            }
        }
        Request::Stop => {}
    }
    Ok(())
}

fn narrate_lines_for(lines: &[RecipientLine], viewer: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in lines {
        if line.to == viewer && !out.contains(&line.text) {
            out.push(line.text.clone());
        }
    }
    out
}

fn restart_idle_timer(restart: &tokio::sync::watch::Sender<u64>) {
    let next = restart.borrow().wrapping_add(1);
    let _ = restart.send(next);
}

fn refresh_players(world: &World, seats: &mut Seats, outputs: &[RoundOutput], out: &mut dyn Write) {
    let reports: Vec<_> = outputs.iter().flat_map(|o| o.reports.clone()).collect();
    let presence = seats.presence();
    let mut failed = Vec::new();
    for player in &seats.players {
        let bytes = match refresh_lines(world, &presence, &player.actor_id) {
            Ok(text) => {
                let mut out = frames::player_report_lines(&reports, &player.actor_id);
                out.push_str(&text);
                for output in outputs {
                    for line in narrate_lines_for(&output.lines, &player.actor_id) {
                        out.push_str(&frames::player_lines(&line));
                    }
                    for item in &output.seen {
                        if item.viewer == player.actor_id && !item.line.is_empty() {
                            out.push_str(&frames::player_lines(&item.line));
                        }
                    }
                }
                out.into_bytes()
            }
            Err(code) => refused_line(code).into_bytes(),
        };
        if !super::outbound::try_send_frame(&player.out, bytes) {
            failed.push(player.conn);
        }
    }
    for conn in failed {
        drop_conn(world, seats, conn, out);
    }
}

fn shutdown(world: &World, seats: &mut Seats, out: &mut dyn Write) {
    let conns: Vec<u64> = seats.players.iter().map(|player| player.conn).collect();
    for conn in conns {
        drop_conn(world, seats, conn, out);
    }
    seats.gm = None;
}

fn on_hello(
    world: &World,
    seats: &mut Seats,
    conn: u64,
    line: &str,
    out_tx: OutTx,
    cancel: CancelTx,
    out: &mut dyn Write,
) -> HelloReply {
    if line == "gm" {
        return HelloReply::Refused(refused_line(Reason::NotInSpec));
    }
    if line.is_empty() {
        if seats.players.len() >= MAX_PLAYER_SEATS {
            return HelloReply::Refused(refused_line(Reason::TableFull));
        }
        return HelloReply::Refused(frames::player_refused_line(
            Reason::ChoosePc.as_str(),
            Some(Missing::Many(world.free_pcs(&seats.presence()))),
        ));
    }
    let Some(id) = read_hello(line) else {
        return HelloReply::Refused(refused_line(Reason::Schema));
    };
    if seats.players.len() >= MAX_PLAYER_SEATS {
        return HelloReply::Refused(refused_line(Reason::TableFull));
    }
    if seats.players.iter().any(|player| player.actor_id == id) {
        return HelloReply::Refused(refused_line(Reason::SeatTaken));
    }
    if !world
        .state()
        .actors
        .get(id)
        .is_some_and(|actor| actor.is_pc())
    {
        return HelloReply::Refused(refused_line(Reason::Schema));
    }
    // Seat first, so the welcome view counts this player as present.
    seats.players.push(PlayerSeat {
        conn,
        actor_id: id.to_string(),
        out: out_tx,
        cancel,
    });
    let sent = welcome(world, &seats.presence(), id).and_then(|bytes| {
        let seated = seats
            .player(conn)
            .is_some_and(|player| super::outbound::try_send_frame(&player.out, bytes));
        if seated {
            Ok(())
        } else {
            Err(Reason::Schema)
        }
    });
    if let Err(code) = sent {
        drop_conn(world, seats, conn, out);
        return HelloReply::Refused(refused_line(code));
    }
    write_digest(out, world, &[]);
    HelloReply::Line
}

fn seat_gm(seats: &mut Seats, conn: u64, out: OutTx, cancel: CancelTx) -> Result<(), Reason> {
    if seats.gm.is_some() {
        return Err(Reason::TableFull);
    }
    seats.gm = Some(GmSeat { conn, out, cancel });
    Ok(())
}

fn read_hello(line: &str) -> Option<&str> {
    crate::protocol::parse_player_hello(line)
}

fn welcome(world: &World, presence: &Presence, actor_id: &str) -> Result<Vec<u8>, Reason> {
    let mut out = String::new();
    for stored in world.player_scrollback(actor_id)? {
        out.push_str(&frames::say_line(&stored, true));
    }
    out.push_str(&refresh_lines(world, presence, actor_id)?);
    Ok(out.into_bytes())
}

fn refresh_lines(world: &World, presence: &Presence, actor_id: &str) -> Result<String, Reason> {
    let view = world.perceive(presence, actor_id)?;
    let mut out = frames::player_resume_line(&view);
    out.push_str(&frames::player_perceive_line(&view));
    Ok(out)
}

enum PlayerCommand<'w, 'l> {
    Speech(Address<'w>, &'l str),
    Lines(String),
    Move(&'l str, Option<&'l str>),
}

fn parse_player_command<'w, 'l: 'w>(
    world: &'w World,
    presence: &Presence,
    seated: &str,
    line: &'l str,
) -> Result<PlayerCommand<'w, 'l>, Reason> {
    let (verb, rest) = split_verb(line, seated)?;
    if verb.is_empty() {
        return Err(Reason::Schema);
    }
    if verb == "say" {
        if rest.is_empty() {
            return Err(Reason::Schema);
        }
        let place = world
            .state()
            .actors
            .get(seated)
            .filter(|actor| actor.is_pc())
            .ok_or(Reason::Schema)?
            .place();
        let (target, text) = split_token(rest);
        let (heard, text) = if !text.is_empty() && target == "place" {
            (Address::Place(place), text)
        } else if !text.is_empty() && world.state().actors.contains_key(target) {
            (Address::Actor(target), text)
        } else {
            (Address::Table, rest)
        };
        return Ok(PlayerCommand::Speech(heard, text));
    }
    if rest.is_empty() {
        return Ok(match verb {
            "help" => PlayerCommand::Lines(world.help_lines(presence, seated)),
            "map" => PlayerCommand::Lines(world.map_lines(presence, seated)),
            "look" => PlayerCommand::Lines(world.look_lines(presence, seated)),
            _ => PlayerCommand::Move(verb, None),
        });
    }
    let (target, extra) = split_token(rest);
    if target.is_empty() || !extra.is_empty() {
        return Err(Reason::Schema);
    }
    Ok(PlayerCommand::Move(verb, Some(target)))
}

fn split_verb<'a>(line: &'a str, seated: &str) -> Result<(&'a str, &'a str), Reason> {
    if line.is_empty() {
        return Err(Reason::Schema);
    }
    let (first, rest) = split_token(line);
    if let Some(id) = first.strip_prefix("player:") {
        if id.is_empty() || id != seated || rest.is_empty() {
            return Err(Reason::Schema);
        }
        return Ok(split_token(rest));
    }
    Ok((first, rest))
}

fn split_token(line: &str) -> (&str, &str) {
    match line.split_once(' ') {
        Some((token, rest)) => (token, rest),
        None => (line, ""),
    }
}

fn on_player(
    world: &mut World,
    seats: &mut Seats,
    restart: &tokio::sync::watch::Sender<u64>,
    conn: u64,
    line: &str,
    out: &mut dyn Write,
) {
    if split_token(line) == ("close", "") {
        drop_conn(world, seats, conn, out);
        return;
    }
    let Some(actor_id) = seats.player(conn).map(|player| player.actor_id.clone()) else {
        return;
    };
    let presence = seats.presence();
    match parse_player_command(world, &presence, &actor_id, line) {
        Ok(PlayerCommand::Speech(heard, text)) => {
            match world.say_as_player(&presence, &actor_id, heard, text) {
                Ok(stored) => {
                    let spoken = frames::SayOutput::from(&stored);
                    let _ = writeln!(out, "{}", spoken.tape_line);
                    let _ = out.flush();
                    send_gm(seats, spoken.gm_event);
                    send_recipients(
                        world,
                        seats,
                        &spoken.recipients,
                        spoken.player_line.into_bytes(),
                        out,
                    );
                }
                Err(code) => send_player(world, seats, conn, refused_line(code).into_bytes(), out),
            }
        }
        Ok(PlayerCommand::Lines(text)) => {
            send_player(
                world,
                seats,
                conn,
                frames::player_lines(&text).into_bytes(),
                out,
            );
        }
        Ok(PlayerCommand::Move(verb, target)) => {
            on_move(world, seats, restart, conn, &actor_id, (verb, target), out);
        }
        Err(code) => send_player(world, seats, conn, refused_line(code).into_bytes(), out),
    }
}

/// With a GM seated, a line waits for `commit_round`. Without one it commits its
/// own round, and a busy PC's line plays the rounds its action still needs.
fn on_move(
    world: &mut World,
    seats: &mut Seats,
    restart: &tokio::sync::watch::Sender<u64>,
    conn: u64,
    actor_id: &str,
    (verb, target): (&str, Option<&str>),
    out: &mut dyn Write,
) {
    let gm_seated = seats.gm.is_some();
    let presence = seats.presence();
    let busy = match world.queue_player_move(&presence, actor_id, verb, target) {
        Ok(queued) if gm_seated => {
            send_gm(
                seats,
                frames::queued_event(&queued.id, &queued.actor_id, &queued.move_name),
            );
            let line = frames::player_queued_line(&queued.id).into_bytes();
            send_player(world, seats, conn, line, out);
            return;
        }
        Ok(_) => None,
        Err(refusal) if !gm_seated && refusal.reason() == Reason::Busy => Some(refusal),
        Err(refusal) => {
            send_player(world, seats, conn, refusal_line(&refusal), out);
            return;
        }
    };
    let outputs = match commit_then_fast_forward(world, &presence) {
        Ok(outputs) => outputs,
        Err(code) => {
            send_player(world, seats, conn, refused_line(code).into_bytes(), out);
            return;
        }
    };
    match &busy {
        Some(refusal) => send_player(world, seats, conn, refusal_line(refusal), out),
        None if !outputs.is_empty() => restart_idle_timer(restart),
        None => {}
    }
    if !outputs.is_empty() {
        write_digest(out, world, &outputs);
        refresh_players(world, seats, &outputs, out);
    }
}

fn refusal_line(refusal: &GateRefusal) -> Vec<u8> {
    let missing = refusal.missing().map(|text| Missing::One(text.to_string()));
    frames::player_refused_line(refusal.token(), missing).into_bytes()
}

/// A full or closed queue drops that seat. The world thread does not wait.
fn send_player(world: &World, seats: &mut Seats, conn: u64, bytes: Vec<u8>, out: &mut dyn Write) {
    let failed = match seats.player(conn) {
        Some(player) => !super::outbound::try_send_frame(&player.out, bytes),
        None => false,
    };
    if failed {
        drop_conn(world, seats, conn, out);
    }
}

fn send_gm(seats: &mut Seats, frame: Vec<u8>) {
    let failed = match seats.gm.as_ref() {
        Some(gm) => !super::outbound::try_send_frame(&gm.out, frame),
        None => false,
    };
    if failed {
        seats.gm = None;
    }
}

fn send_recipients(
    world: &World,
    seats: &mut Seats,
    recipients: &[String],
    line: Vec<u8>,
    out: &mut dyn Write,
) {
    let mut failed = Vec::new();
    for player in &seats.players {
        if recipients.iter().any(|id| id == &player.actor_id)
            && !super::outbound::try_send_frame(&player.out, line.clone())
        {
            failed.push(player.conn);
        }
    }
    for conn in failed {
        drop_conn(world, seats, conn, out);
    }
}

/// Drops a seated player and tells the host. Dropping the seat cancels its socket.
fn drop_conn(world: &World, seats: &mut Seats, conn: u64, out: &mut dyn Write) {
    if seats.take_player(conn).is_some() {
        write_digest(out, world, &[]);
    }
}

fn on_hangup(
    world: &World,
    seats: &mut Seats,
    conn: u64,
    ack: tokio::sync::oneshot::Sender<()>,
    out: &mut dyn Write,
) {
    drop_conn(world, seats, conn, out);
    if seats.gm.as_ref().is_some_and(|gm| gm.conn == conn) {
        seats.gm = None;
    }
    let _ = ack.send(());
}

pub(super) fn refused_line(code: Reason) -> String {
    frames::player_refused_line(code.as_str(), None)
}
