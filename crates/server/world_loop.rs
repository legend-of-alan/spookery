//! The table loop: seat players, read lines, and answer the GM.

use std::io::Write;

use spookery_engine::{GateRefusal, Heard, Presence, Reason, RecipientLine, RoundOutput, World};

use crate::protocol::Missing;

use super::frames;
use super::idle_clock::{self, wall_now};
use super::limits::{CancelTx, OutTx};

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

pub(super) fn run_world_loop(
    mut world: World,
    rx: std::sync::mpsc::Receiver<WorldMsg>,
    restart: &tokio::sync::watch::Sender<u64>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<(), super::listen::Error> {
    let mut seats = Seats::default();
    loop {
        let msg = rx
            .recv()
            .map_err(|_| super::listen::Error::Reason(Reason::NotInSpec))?;
        if matches!(msg, WorldMsg::Stop) {
            shutdown(&world, &mut seats, out);
            return Ok(());
        }
        let handled = handle(&mut world, &mut seats, restart, out, msg);
        if let Some(warning) = world.take_warning() {
            let _ = writeln!(err, "{warning}");
            let _ = err.flush();
        }
        handled.map_err(super::listen::Error::Reason)?;
    }
}

fn write_host(out: &mut dyn Write, world: &World, outputs: &[RoundOutput]) {
    let text = world.host_lines(outputs);
    if !text.is_empty() {
        let _ = out.write_all(text.as_bytes());
        let _ = out.flush();
    }
}

/// The one owner of who is seated. Rounds and readers take `presence()`.
#[derive(Default)]
struct Seats {
    players: Vec<HeldPlayer>,
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
            humans: actors_present.len() as u32,
            actors_present,
        }
    }

    fn player(&self, conn: u64) -> Option<&HeldPlayer> {
        self.players.iter().find(|player| player.conn == conn)
    }

    fn take_player(&mut self, conn: u64) -> Option<HeldPlayer> {
        let index = self.players.iter().position(|player| player.conn == conn)?;
        Some(self.players.remove(index))
    }
}

struct HeldPlayer {
    conn: u64,
    actor_id: String,
    out: OutTx,
    cancel: CancelTx,
}

impl Drop for HeldPlayer {
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

pub(super) enum WorldMsg {
    Hello {
        conn: u64,
        line: String,
        out: OutTx,
        cancel: CancelTx,
        reply: tokio::sync::oneshot::Sender<HelloReply>,
    },
    Player {
        conn: u64,
        line: String,
        reply: tokio::sync::oneshot::Sender<()>,
    },
    Fault {
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
    Claim {
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
    Player,
}

fn handle(
    world: &mut World,
    seats: &mut Seats,
    restart: &tokio::sync::watch::Sender<u64>,
    out: &mut dyn Write,
    msg: WorldMsg,
) -> Result<(), Reason> {
    match msg {
        WorldMsg::Hello {
            conn,
            line,
            out: out_tx,
            cancel,
            reply,
        } => {
            let answer = on_hello(world, seats, conn, &line, out_tx, cancel, out);
            let _ = reply.send(answer);
        }
        WorldMsg::Claim {
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
        WorldMsg::Player { conn, line, reply } => {
            on_player(world, seats, restart, conn, &line, out);
            let _ = reply.send(());
        }
        WorldMsg::Fault { conn, ack } => on_fault(world, seats, conn, ack, out),
        WorldMsg::Tool {
            conn,
            name,
            arguments,
            reply,
        } => {
            if seats.gm.as_ref().is_none_or(|gm| gm.conn != conn) {
                let _ = reply.send(crate::protocol::code_result(Reason::NotInSpec.token()));
                return Ok(());
            }
            let presence = seats.presence();
            let mut done = super::gm_tools::call_gm_tool(world, &presence, &name, arguments);
            if let Some(code) = &done.refused {
                let _ = writeln!(out, "refused {name} {}", code.token());
                let _ = out.flush();
            }
            let outputs = std::mem::take(&mut done.outputs);
            let reports: Vec<_> = outputs.iter().flat_map(|o| o.reports.clone()).collect();
            if !outputs.is_empty() {
                refresh_players(world, seats, &outputs, out);
                write_host(out, world, &outputs);
                bump_sleep(restart);
            }
            if let Some(tape) = &done.tape {
                let _ = writeln!(out, "{tape}");
                let _ = out.flush();
            }
            if let Some((recipients, line)) = &done.player_line {
                send_recipients(world, seats, recipients, line.clone(), out);
            }
            let mut after = done.after;
            after.extend(frames::gm_check_frames(&reports));
            for frame in after {
                send_gm(seats, frame);
            }
            // Free the seat before the client can read this result.
            if done.release_seat {
                seats.gm = None;
            }
            let _ = reply.send(done.value);
        }
        WorldMsg::Wake => {
            if idle_clock::on_scheduled_wake(world, !seats.players.is_empty(), seats.gm.is_some())?
            {
                bump_sleep(restart);
            }
        }
        WorldMsg::Stop => {}
    }
    Ok(())
}

fn narrate_for(lines: &[RecipientLine], viewer: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in lines {
        if line.to == viewer && !out.contains(&line.text) {
            out.push(line.text.clone());
        }
    }
    out
}

fn bump_sleep(restart: &tokio::sync::watch::Sender<u64>) {
    let next = restart.borrow().wrapping_add(1);
    let _ = restart.send(next);
}

fn refresh_players(world: &World, seats: &mut Seats, outputs: &[RoundOutput], out: &mut dyn Write) {
    let reports: Vec<_> = outputs.iter().flat_map(|o| o.reports.clone()).collect();
    let presence = seats.presence();
    let mut failed = Vec::new();
    for player in &seats.players {
        let bytes = match player_refresh(world, &presence, &player.actor_id) {
            Ok(text) => {
                let mut out = frames::player_report_lines(&reports, &player.actor_id);
                out.push_str(&text);
                for output in outputs {
                    for line in narrate_for(&output.lines, &player.actor_id) {
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
        if !super::limits::try_frame(&player.out, bytes) {
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
            return HelloReply::Refused(refused_line(Reason::SessionFull));
        }
        return HelloReply::Refused(frames::player_refused(
            Reason::ChoosePc.token(),
            Some(Missing::Many(world.free_pcs(&seats.presence()))),
        ));
    }
    let Some(id) = player_hello(line) else {
        return HelloReply::Refused(refused_line(Reason::Schema));
    };
    if seats.players.len() >= MAX_PLAYER_SEATS {
        return HelloReply::Refused(refused_line(Reason::SessionFull));
    }
    if seats.players.iter().any(|player| player.actor_id == id) {
        return HelloReply::Refused(refused_line(Reason::SeatTaken));
    }
    if !world
        .now()
        .actors
        .get(id)
        .is_some_and(|actor| actor.is_pc())
    {
        return HelloReply::Refused(refused_line(Reason::Schema));
    }
    // Seat first, so the welcome view counts this player as present.
    seats.players.push(HeldPlayer {
        conn,
        actor_id: id.to_string(),
        out: out_tx,
        cancel,
    });
    let sent = welcome(world, &seats.presence(), id).and_then(|bytes| {
        let seated = seats
            .player(conn)
            .is_some_and(|player| super::limits::try_frame(&player.out, bytes));
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
    write_host(out, world, &[]);
    HelloReply::Player
}

fn seat_gm(seats: &mut Seats, conn: u64, out: OutTx, cancel: CancelTx) -> Result<(), Reason> {
    if seats.gm.is_some() {
        return Err(Reason::SessionFull);
    }
    seats.gm = Some(GmSeat { conn, out, cancel });
    Ok(())
}

fn player_hello(line: &str) -> Option<&str> {
    crate::protocol::read_player_hello(line)
}

fn welcome(world: &World, presence: &Presence, actor_id: &str) -> Result<Vec<u8>, Reason> {
    let mut out = String::new();
    for stored in world.player_scrollback(actor_id)? {
        out.push_str(&frames::say_line(&stored, true));
    }
    out.push_str(&player_refresh(world, presence, actor_id)?);
    Ok(out.into_bytes())
}

fn player_refresh(world: &World, presence: &Presence, actor_id: &str) -> Result<String, Reason> {
    let view = world.perceive(presence, actor_id)?;
    let mut out = frames::player_resume_line(&view);
    out.push_str(&frames::player_perceive_line(&view));
    Ok(out)
}

enum PlayerCommand<'w, 'l> {
    Speech(Heard<'w>, &'l str),
    Lines(String),
    Turn(&'l str, Option<&'l str>),
}

fn parse_player_command<'w, 'l: 'w>(
    world: &'w World,
    presence: &Presence,
    seated: &str,
    line: &'l str,
) -> Result<PlayerCommand<'w, 'l>, Reason> {
    let (verb, rest) = verb_line(line, seated)?;
    if verb.is_empty() {
        return Err(Reason::Schema);
    }
    if verb == "say" {
        if rest.is_empty() {
            return Err(Reason::Schema);
        }
        let place = world
            .now()
            .actors
            .get(seated)
            .filter(|actor| actor.is_pc())
            .ok_or(Reason::Schema)?
            .place();
        let (target, text) = split_token(rest);
        let (heard, text) = if !text.is_empty() && target == "place" {
            (Heard::Place(place), text)
        } else if !text.is_empty() && world.now().actors.contains_key(target) {
            (Heard::Actor(target), text)
        } else {
            (Heard::Table, rest)
        };
        return Ok(PlayerCommand::Speech(heard, text));
    }
    if rest.is_empty() {
        return Ok(match verb {
            "help" => PlayerCommand::Lines(world.help_lines(presence, seated)),
            "map" => PlayerCommand::Lines(world.player_lines(presence, seated)),
            "look" => PlayerCommand::Lines(world.look_lines(presence, seated)),
            _ => PlayerCommand::Turn(verb, None),
        });
    }
    let (target, extra) = split_token(rest);
    if target.is_empty() || !extra.is_empty() {
        return Err(Reason::Schema);
    }
    Ok(PlayerCommand::Turn(verb, Some(target)))
}

fn verb_line<'a>(line: &'a str, seated: &str) -> Result<(&'a str, &'a str), Reason> {
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
            match world.speak_player(&presence, &actor_id, heard, text) {
                Ok(stored) => {
                    let spoken = frames::spoken_from(&stored);
                    let _ = writeln!(out, "{}", spoken.tape);
                    let _ = out.flush();
                    send_gm(seats, spoken.gm_event);
                    send_recipients(
                        world,
                        seats,
                        &spoken.recipients,
                        spoken.player_event.into_bytes(),
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
        Ok(PlayerCommand::Turn(verb, target)) => {
            on_mechanical(world, seats, restart, conn, &actor_id, (verb, target), out);
        }
        Err(code) => send_player(world, seats, conn, refused_line(code).into_bytes(), out),
    }
}

/// With a GM seated, a line waits for `commit_round`. Without one it commits its
/// own round, and a busy PC's line plays the rounds its action still needs.
fn on_mechanical(
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
    match world.queue_player_move(&presence, actor_id, verb, target) {
        Ok(queued) if gm_seated => {
            send_gm(
                seats,
                frames::queued_event(&queued.id, &queued.actor_id, &queued.move_name),
            );
            send_player(
                world,
                seats,
                conn,
                frames::player_queued_line(&queued.id).into_bytes(),
                out,
            );
        }
        Ok(_) => match commit_then_fast_forward(world, &presence) {
            Ok(outputs) => {
                if !outputs.is_empty() {
                    bump_sleep(restart);
                }
                write_host(out, world, &outputs);
                refresh_players(world, seats, &outputs, out);
            }
            Err(code) => send_player(world, seats, conn, refused_line(code).into_bytes(), out),
        },
        Err(busy) if !gm_seated && busy.reason() == Reason::Busy => {
            match commit_then_fast_forward(world, &presence) {
                Ok(outputs) => {
                    send_player(world, seats, conn, refusal_line(&busy), out);
                    if !outputs.is_empty() {
                        refresh_players(world, seats, &outputs, out);
                        write_host(out, world, &outputs);
                    }
                }
                Err(code) => send_player(world, seats, conn, refused_line(code).into_bytes(), out),
            }
        }
        Err(refusal) => send_player(world, seats, conn, refusal_line(&refusal), out),
    }
}

fn refusal_line(refusal: &GateRefusal) -> Vec<u8> {
    let missing = refusal.missing().map(|text| Missing::One(text.to_string()));
    frames::player_refused(refusal.token(), missing).into_bytes()
}

/// A full or closed queue drops that seat. The world thread does not wait.
fn send_player(world: &World, seats: &mut Seats, conn: u64, bytes: Vec<u8>, out: &mut dyn Write) {
    let failed = match seats.player(conn) {
        Some(player) => !super::limits::try_frame(&player.out, bytes),
        None => false,
    };
    if failed {
        drop_conn(world, seats, conn, out);
    }
}

fn send_gm(seats: &mut Seats, frame: Vec<u8>) {
    let failed = match seats.gm.as_ref() {
        Some(gm) => !super::limits::try_frame(&gm.out, frame),
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
            && !super::limits::try_frame(&player.out, line.clone())
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
        write_host(out, world, &[]);
    }
}

fn on_fault(
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

fn refused_line(code: Reason) -> String {
    frames::player_refused(code.token(), None)
}
