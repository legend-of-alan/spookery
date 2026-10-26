//! The table loop: seat players, read lines, and answer the GM.

use std::io::Write;

pub use spookery_engine::MoveOutcome;
use spookery_engine::{Heard, Reason, RecipientLine, RoundOutput, World, MAX_PLAYER_SEATS};

use crate::protocol::Missing;

use super::frames;
use super::idle_clock::{self, wall_now};
use super::limits::{CancelTx, OutTx};

/// Commit a queued round, then fast-forward while every seated PC is busy.
/// An empty queue only fast-forwards.
pub fn commit_then_fast_forward(world: &mut World) -> Result<Vec<RoundOutput>, Reason> {
    let wall = wall_now()?;
    world.commit_then_fast_forward(&wall)
}

pub(super) fn run_world_loop(
    mut world: World,
    rx: std::sync::mpsc::Receiver<WorldMsg>,
    restart: &tokio::sync::watch::Sender<u64>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<(), super::listen::Error> {
    let mut seats = Seats::default();
    debug_seats(&seats, &world);
    loop {
        let msg = rx
            .recv()
            .map_err(|_| super::listen::Error::Reason(Reason::NotInSpec))?;
        if matches!(msg, WorldMsg::Stop) {
            return shutdown(&mut world, &mut seats, out).map_err(super::listen::Error::Reason);
        }
        let handled = handle(&mut world, &mut seats, restart, out, msg);
        if let Some(warning) = world.take_warning() {
            let _ = writeln!(err, "{warning}");
            let _ = err.flush();
        }
        handled.map_err(super::listen::Error::Reason)?;
        debug_seats(&seats, &world);
    }
}

pub(super) fn write_host(out: &mut dyn Write, world: &World, outputs: &[RoundOutput]) {
    let text = world.host_lines(outputs);
    if !text.is_empty() {
        let _ = out.write_all(text.as_bytes());
        let _ = out.flush();
    }
}

struct Seats {
    players: PlayerSeats,
    gm: Option<GmSeat>,
}

struct PlayerSeats {
    slots: [Option<HeldPlayer>; MAX_PLAYER_SEATS],
}

impl Default for Seats {
    fn default() -> Self {
        Self {
            players: PlayerSeats {
                slots: [None, None, None],
            },
            gm: None,
        }
    }
}

impl PlayerSeats {
    fn len(&self) -> usize {
        self.slots.iter().flatten().count()
    }

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn actor_ids(&self) -> Vec<String> {
        self.slots
            .iter()
            .flatten()
            .map(|player| player.actor_id.clone())
            .collect()
    }

    fn conns(&self) -> Vec<u64> {
        self.slots
            .iter()
            .flatten()
            .map(|player| player.conn)
            .collect()
    }

    fn by_conn(&self, conn: u64) -> Option<&HeldPlayer> {
        self.slots
            .iter()
            .flatten()
            .find(|player| player.conn == conn)
    }

    fn sit(&mut self, player: HeldPlayer) -> Result<(), Reason> {
        let Some(slot) = self.slots.iter_mut().find(|slot| slot.is_none()) else {
            return Err(Reason::SessionFull);
        };
        *slot = Some(player);
        Ok(())
    }

    fn take_conn(&mut self, conn: u64) -> Option<HeldPlayer> {
        for slot in &mut self.slots {
            if slot.as_ref().is_some_and(|player| player.conn == conn) {
                return slot.take();
            }
        }
        None
    }
}

struct HeldPlayer {
    conn: u64,
    actor_id: String,
    session_id: String,
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
        reply: tokio::sync::oneshot::Sender<PlayerReply>,
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

pub(super) enum PlayerReply {
    Done,
    Hangup,
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
            let answer = on_hello(world, seats, conn, &line, out_tx, cancel, out)?;
            let _ = reply.send(answer);
        }
        WorldMsg::Claim {
            conn,
            out: out_tx,
            cancel,
            reply,
        } => match seat_gm(seats, conn, out_tx, cancel) {
            Ok(()) => {
                if reply.send(Ok(())).is_ok() {
                    send_gm(seats, frames::pack_event(world));
                } else {
                    seats.gm = None;
                }
            }
            Err(code) => {
                let _ = reply.send(Err(code));
            }
        },
        WorldMsg::Player { conn, line, reply } => {
            let answer = on_player(world, seats, restart, conn, &line, out)?;
            let _ = reply.send(answer);
        }
        WorldMsg::Fault { conn, ack } => on_fault(world, seats, conn, ack, out)?,
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
            let mut done = super::gm_tools::call_gm_tool(world, &name, arguments);
            if let Some(code) = &done.refused {
                let _ = writeln!(out, "refused {name} {}", code.token());
                let _ = out.flush();
            }
            let outputs = std::mem::take(&mut done.outputs);
            let reports: Vec<_> = outputs.iter().flat_map(|o| o.reports.clone()).collect();
            if done.ticked {
                refresh_players(world, seats, &outputs, out)?;
                write_host(out, world, &outputs);
            }
            if let Some(tape) = &done.tape {
                let _ = writeln!(out, "{tape}");
                let _ = out.flush();
            }
            if let Some(frame) = done.gm_event {
                send_gm(seats, frame);
            }
            if let Some((recipients, line)) = &done.player_line {
                send_recipients(world, seats, recipients, line.clone(), out)?;
            }
            let mut after = done.after;
            after.extend(frames::gm_check_frames(&reports));
            for frame in after {
                send_gm(seats, frame);
            }
            if done.advanced {
                bump_sleep(restart);
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

fn refresh_players(
    world: &mut World,
    seats: &mut Seats,
    outputs: &[RoundOutput],
    out: &mut dyn Write,
) -> Result<(), Reason> {
    let reports: Vec<_> = outputs.iter().flat_map(|o| o.reports.clone()).collect();
    let mut failed = Vec::new();
    for player in seats.players.slots.iter().flatten() {
        let bytes = match player_refresh(world, &player.actor_id) {
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
        if drop_conn(world, seats, conn)? {
            write_host(out, world, &[]);
        }
    }
    Ok(())
}

fn shutdown(world: &mut World, seats: &mut Seats, out: &mut dyn Write) -> Result<(), Reason> {
    // The testing feature is off in shipped binaries. Tests set the path so a
    // second interrupt can land while `join` is still blocked.
    #[cfg(feature = "testing")]
    if let Ok(path) = std::env::var("SPOOK_SHUTDOWN_HOLD") {
        let _ = std::fs::write(path, b"held\n");
        loop {
            std::thread::park();
        }
    }
    for conn in seats.players.conns() {
        if drop_conn(world, seats, conn)? {
            write_host(out, world, &[]);
        }
    }
    seats.gm = None;
    Ok(())
}

fn on_hello(
    world: &mut World,
    seats: &mut Seats,
    conn: u64,
    line: &str,
    out_tx: OutTx,
    cancel: CancelTx,
    out: &mut dyn Write,
) -> Result<HelloReply, Reason> {
    if line == "gm" {
        return Ok(HelloReply::Refused(refused_line(Reason::NotInSpec)));
    }
    if line.is_empty() {
        if seats.players.len() >= MAX_PLAYER_SEATS {
            return Ok(HelloReply::Refused(refused_line(Reason::SessionFull)));
        }
        return Ok(HelloReply::Refused(frames::player_refused(
            Reason::ChoosePc.token(),
            Some(Missing::Many(world.free_pcs())),
        )));
    }
    if let Some(id) = player_hello(line) {
        let session_id = match world.seat_player(id) {
            Ok(session_id) => session_id,
            Err(code) => return Ok(HelloReply::Refused(refused_line(code))),
        };
        let res = welcome(world, id).and_then(|bytes| {
            if !super::limits::try_frame(&out_tx, bytes) {
                return Err(Reason::Schema);
            }
            seats.players.sit(HeldPlayer {
                conn,
                actor_id: id.to_string(),
                session_id: session_id.clone(),
                out: out_tx,
                cancel,
            })
        });
        if let Err(code) = res {
            if world.release_player_seat(&session_id, id)? {
                write_host(out, world, &[]);
            }
            return Ok(HelloReply::Refused(refused_line(code)));
        }
        write_host(out, world, &[]);
        return Ok(HelloReply::Player);
    }
    Ok(HelloReply::Refused(refused_line(Reason::Schema)))
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

fn welcome(world: &World, actor_id: &str) -> Result<Vec<u8>, Reason> {
    let mut out = String::new();
    for stored in world.player_scrollback(actor_id)? {
        out.push_str(&frames::player_replay_line(&stored));
    }
    out.push_str(&player_refresh(world, actor_id)?);
    Ok(out.into_bytes())
}

fn player_refresh(world: &World, actor_id: &str) -> Result<String, Reason> {
    let view = world.perceive(actor_id)?;
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
            "help" => PlayerCommand::Lines(world.help_lines(seated)),
            "map" => PlayerCommand::Lines(world.player_lines(seated)),
            "look" => PlayerCommand::Lines(world.look_lines(seated)),
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
) -> Result<PlayerReply, Reason> {
    if split_token(line) == ("close", "") {
        if drop_conn(world, seats, conn)? {
            write_host(out, world, &[]);
        }
        return Ok(PlayerReply::Hangup);
    }
    let Some(actor_id) = seats
        .players
        .by_conn(conn)
        .map(|player| player.actor_id.clone())
    else {
        return Ok(PlayerReply::Hangup);
    };
    match parse_player_command(world, &actor_id, line) {
        Ok(PlayerCommand::Speech(heard, text)) => {
            match world.speak_player(&actor_id, heard, text) {
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
                    )?;
                }
                Err(code) => send_player(world, seats, conn, refused_line(code).into_bytes(), out)?,
            }
        }
        Ok(PlayerCommand::Lines(text)) => {
            send_player(
                world,
                seats,
                conn,
                frames::player_lines(&text).into_bytes(),
                out,
            )?;
        }
        Ok(PlayerCommand::Turn(verb, target)) => {
            on_mechanical(world, seats, restart, conn, &actor_id, (verb, target), out)?;
        }
        Err(code) => send_player(world, seats, conn, refused_line(code).into_bytes(), out)?,
    }
    Ok(PlayerReply::Done)
}

fn on_mechanical(
    world: &mut World,
    seats: &mut Seats,
    restart: &tokio::sync::watch::Sender<u64>,
    conn: u64,
    actor_id: &str,
    (verb, target): (&str, Option<&str>),
    out: &mut dyn Write,
) -> Result<PlayerReply, Reason> {
    let gm_seated = seats.gm.is_some();
    match spookery_engine::handle_player_move(
        world,
        actor_id,
        verb,
        target,
        gm_seated,
        &wall_now()?,
    ) {
        Ok(MoveOutcome::Queued(queued)) => {
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
            )?;
        }
        Ok(MoveOutcome::Committed { advanced, outputs }) => {
            if advanced {
                bump_sleep(restart);
            }
            write_host(out, world, &outputs);
            refresh_players(world, seats, &outputs, out)?;
        }
        Ok(MoveOutcome::Busy {
            code,
            missing,
            outputs,
        }) => {
            send_player(
                world,
                seats,
                conn,
                frames::player_refused(code.token(), missing.map(Missing::One)).into_bytes(),
                out,
            )?;
            if !outputs.is_empty() {
                refresh_players(world, seats, &outputs, out)?;
                write_host(out, world, &outputs);
            }
        }
        Err(refusal) => send_player(
            world,
            seats,
            conn,
            frames::player_refused(
                refusal.token(),
                refusal.missing().map(|text| Missing::One(text.to_string())),
            )
            .into_bytes(),
            out,
        )?,
    }
    Ok(PlayerReply::Done)
}

/// A full or closed queue drops that seat. The world thread does not wait.
fn send_player(
    world: &mut World,
    seats: &mut Seats,
    conn: u64,
    bytes: Vec<u8>,
    out: &mut dyn Write,
) -> Result<(), Reason> {
    let failed = match seats.players.by_conn(conn) {
        Some(player) => !super::limits::try_frame(&player.out, bytes),
        None => false,
    };
    if failed && drop_conn(world, seats, conn)? {
        write_host(out, world, &[]);
    }
    Ok(())
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
    world: &mut World,
    seats: &mut Seats,
    recipients: &[String],
    line: Vec<u8>,
    out: &mut dyn Write,
) -> Result<(), Reason> {
    let mut failed = Vec::new();
    for player in seats.players.slots.iter().flatten() {
        if recipients.iter().any(|id| id == &player.actor_id)
            && !super::limits::try_frame(&player.out, line.clone())
        {
            failed.push(player.conn);
        }
    }
    for conn in failed {
        if drop_conn(world, seats, conn)? {
            write_host(out, world, &[]);
        }
    }
    Ok(())
}

fn drop_conn(world: &mut World, seats: &mut Seats, conn: u64) -> Result<bool, Reason> {
    let Some(player) = seats.players.take_conn(conn) else {
        return Ok(false);
    };
    world.release_player_seat(&player.session_id, &player.actor_id)
}

fn on_fault(
    world: &mut World,
    seats: &mut Seats,
    conn: u64,
    ack: tokio::sync::oneshot::Sender<()>,
    out: &mut dyn Write,
) -> Result<(), Reason> {
    let res = drop_conn(world, seats, conn);
    if matches!(res, Ok(true)) {
        write_host(out, world, &[]);
    } else if seats.gm.as_ref().is_some_and(|gm| gm.conn == conn) {
        seats.gm = None;
    }
    let _ = ack.send(());
    res.map(|_| ())
}

fn refused_line(code: Reason) -> String {
    frames::player_refused(code.token(), None)
}

fn debug_seats(seats: &Seats, world: &World) {
    let mut held = seats.players.actor_ids();
    held.sort();
    let presence = &world.now().presence;
    debug_assert_eq!(presence.humans as usize, held.len());
    let mut present = presence.actors_present.clone();
    present.sort();
    debug_assert_eq!(present, held);
}
