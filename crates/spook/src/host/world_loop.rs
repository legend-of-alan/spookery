//! The table loop: seat players, read lines, and answer the GM.

use std::io::{self, Write};

use spookery_engine::{
    GateRefusal, Heard, QueuedMove, Reason, RecipientLine, RoundOutput, World, MAX_PLAYER_SEATS,
};

use super::frames;
use super::idle_clock::{self, wall_now};
use super::limits::{CancelTx, OutTx};

pub enum MoveOutcome {
    Queued(QueuedMove),
    Committed {
        advanced: bool,
        outputs: Vec<RoundOutput>,
    },
    Busy {
        code: Reason,
        missing: Option<String>,
        outputs: Vec<RoundOutput>,
    },
}

/// Commit a queued round, then fast-forward while every seated PC is busy.
/// An empty queue only fast-forwards.
pub fn commit_then_fast_forward(world: &mut World) -> Result<Vec<RoundOutput>, Reason> {
    let wall = wall_now()?;
    world.commit_then_fast_forward(&wall)
}

pub fn handle_player_move(
    world: &mut World,
    actor_id: &str,
    verb: &str,
    target: Option<&str>,
    gm_seated: bool,
) -> Result<MoveOutcome, GateRefusal> {
    let queued = match world.queue_player_move(actor_id, verb, target) {
        Ok(queued) => queued,
        Err(refusal) => {
            if !gm_seated && refusal.reason() == Reason::Busy {
                let outputs = commit_then_fast_forward(world).map_err(GateRefusal::from)?;
                return Ok(MoveOutcome::Busy {
                    code: refusal.reason(),
                    missing: refusal.missing().map(str::to_string),
                    outputs,
                });
            }
            return Err(refusal);
        }
    };
    if gm_seated {
        return Ok(MoveOutcome::Queued(queued));
    }
    let before = world.now().world_time;
    let outputs = commit_then_fast_forward(world)?;
    Ok(MoveOutcome::Committed {
        advanced: world.now().world_time != before,
        outputs,
    })
}

pub(super) fn run_world_loop(
    mut world: World,
    rx: std::sync::mpsc::Receiver<WorldMsg>,
    restart: &tokio::sync::watch::Sender<u64>,
) -> ! {
    let mut seats = Seats::default();
    debug_seats(&seats, &world);
    loop {
        let msg = match rx.recv() {
            Ok(msg) => msg,
            Err(_) => {
                eprintln!("{}", Reason::NotInSpec);
                std::process::exit(1);
            }
        };
        let handled = handle(&mut world, &mut seats, restart, msg);
        if let Some(warning) = world.take_warning() {
            eprintln!("{warning}");
        }
        if let Err(code) = handled {
            eprintln!("{code}");
            std::process::exit(1);
        }
        debug_seats(&seats, &world);
    }
}

pub(super) fn print_host(world: &World, outputs: &[RoundOutput]) {
    let text = world.host_lines(outputs);
    if !text.is_empty() {
        print!("{text}");
        let _ = io::stdout().flush();
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
    Stdin(String),
    StdinEof,
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
    msg: WorldMsg,
) -> Result<(), Reason> {
    match msg {
        WorldMsg::Hello {
            conn,
            line,
            out,
            cancel,
            reply,
        } => {
            let answer = on_hello(world, seats, conn, &line, out, cancel)?;
            let _ = reply.send(answer);
        }
        WorldMsg::Claim {
            conn,
            out,
            cancel,
            reply,
        } => match seat_gm(seats, conn, out, cancel) {
            Ok(()) if reply.send(Ok(())).is_ok() => send_gm(seats, frames::pack_event(world)),
            Ok(()) => seats.gm = None,
            Err(code) => {
                let _ = reply.send(Err(code));
            }
        },
        WorldMsg::Player { conn, line, reply } => {
            let answer = on_player(world, seats, restart, conn, &line)?;
            let _ = reply.send(answer);
        }
        WorldMsg::Fault { conn, ack } => on_fault(world, seats, conn, ack)?,
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
                println!("refused {name} {}", code.token());
                let _ = io::stdout().flush();
            }
            let outputs = std::mem::take(&mut done.outputs);
            let reports: Vec<_> = outputs.iter().flat_map(|o| o.reports.clone()).collect();
            if done.ticked {
                refresh_players(world, seats, &outputs)?;
                print_host(world, &outputs);
            }
            if let Some(tape) = &done.tape {
                println!("{tape}");
                let _ = io::stdout().flush();
            }
            if let Some(frame) = done.gm_event {
                send_gm(seats, frame);
            }
            if let Some((recipients, line)) = &done.player_line {
                send_recipients(world, seats, recipients, line.clone())?;
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
        WorldMsg::Stdin(line) if line == "stop" => shutdown(world, seats),
        WorldMsg::Stdin(_) => eprintln!("{}", Reason::NotInSpec),
        WorldMsg::StdinEof => shutdown(world, seats),
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
            print_host(world, &[]);
        }
    }
    Ok(())
}

fn shutdown(world: &mut World, seats: &mut Seats) -> ! {
    for conn in seats.players.conns() {
        match drop_conn(world, seats, conn) {
            Ok(true) => print_host(world, &[]),
            Ok(false) => {}
            Err(code) => {
                eprintln!("{code}");
                std::process::exit(1);
            }
        }
    }
    seats.gm = None;
    std::process::exit(0);
}

fn on_hello(
    world: &mut World,
    seats: &mut Seats,
    conn: u64,
    line: &str,
    out: OutTx,
    cancel: CancelTx,
) -> Result<HelloReply, Reason> {
    if line == "gm" {
        return Ok(HelloReply::Refused(refused_line(Reason::NotInSpec)));
    }
    if line.is_empty() || line == "player:" {
        if seats.players.len() >= MAX_PLAYER_SEATS {
            return Ok(HelloReply::Refused(refused_line(Reason::SessionFull)));
        }
        let free = world.free_pcs();
        if free.is_empty() {
            return Ok(HelloReply::Refused(refused_line(Reason::SeatTaken)));
        }
        return Ok(HelloReply::Refused(frames::player_refused(
            Reason::ChoosePc.token(),
            Some(&free.join(", ")),
        )));
    }
    if let Some(id) = player_hello(line) {
        let session_id = match world.seat_player(id) {
            Ok(session_id) => session_id,
            Err(code) => return Ok(HelloReply::Refused(refused_line(code))),
        };
        let bytes = match welcome(world, id) {
            Ok(bytes) => bytes,
            Err(code) => {
                if world.release_player_seat(&session_id, id)? {
                    print_host(world, &[]);
                }
                return Ok(HelloReply::Refused(refused_line(code)));
            }
        };
        if !super::limits::try_frame(&out, bytes) {
            if world.release_player_seat(&session_id, id)? {
                print_host(world, &[]);
            }
            return Ok(HelloReply::Refused(refused_line(Reason::Schema)));
        }
        if let Err(code) = seats.players.sit(HeldPlayer {
            conn,
            actor_id: id.to_string(),
            session_id: session_id.clone(),
            out,
            cancel,
        }) {
            if world.release_player_seat(&session_id, id)? {
                print_host(world, &[]);
            }
            return Ok(HelloReply::Refused(refused_line(code)));
        }
        print_host(world, &[]);
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
    Help,
    Map,
    Look,
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
            "help" => PlayerCommand::Help,
            "map" => PlayerCommand::Map,
            "look" => PlayerCommand::Look,
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
) -> Result<PlayerReply, Reason> {
    if split_token(line) == ("close", "") {
        if drop_conn(world, seats, conn)? {
            print_host(world, &[]);
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
                    println!("{}", spoken.tape);
                    let _ = io::stdout().flush();
                    send_gm(seats, spoken.gm_event);
                    send_recipients(
                        world,
                        seats,
                        &spoken.recipients,
                        spoken.player_event.into_bytes(),
                    )?;
                }
                Err(code) => send_player(world, seats, conn, refused_line(code).into_bytes())?,
            }
        }
        Ok(PlayerCommand::Help) => {
            let text = world.help_lines(&actor_id);
            send_player(world, seats, conn, frames::player_lines(&text).into_bytes())?;
        }
        Ok(PlayerCommand::Map) => {
            let text = world.player_lines(&actor_id);
            send_player(world, seats, conn, frames::player_lines(&text).into_bytes())?;
        }
        Ok(PlayerCommand::Look) => {
            let text = world.look_lines(&actor_id);
            send_player(world, seats, conn, frames::player_lines(&text).into_bytes())?;
        }
        Ok(PlayerCommand::Turn(verb, target)) => {
            on_mechanical(world, seats, restart, conn, &actor_id, verb, target)?;
        }
        Err(code) => send_player(world, seats, conn, refused_line(code).into_bytes())?,
    }
    Ok(PlayerReply::Done)
}

fn on_mechanical(
    world: &mut World,
    seats: &mut Seats,
    restart: &tokio::sync::watch::Sender<u64>,
    conn: u64,
    actor_id: &str,
    verb: &str,
    target: Option<&str>,
) -> Result<PlayerReply, Reason> {
    let gm_seated = seats.gm.is_some();
    match handle_player_move(world, actor_id, verb, target, gm_seated) {
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
            )?;
        }
        Ok(MoveOutcome::Committed { advanced, outputs }) => {
            if advanced {
                bump_sleep(restart);
            }
            print_host(world, &outputs);
            refresh_players(world, seats, &outputs)?;
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
                frames::player_refused(code.token(), missing.as_deref()).into_bytes(),
            )?;
            if !outputs.is_empty() {
                refresh_players(world, seats, &outputs)?;
                print_host(world, &outputs);
            }
        }
        Err(refusal) => send_player(
            world,
            seats,
            conn,
            frames::player_refused(refusal.token(), refusal.missing()).into_bytes(),
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
) -> Result<(), Reason> {
    let failed = match seats.players.by_conn(conn) {
        Some(player) => !super::limits::try_frame(&player.out, bytes),
        None => false,
    };
    if failed && drop_conn(world, seats, conn)? {
        print_host(world, &[]);
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
            print_host(world, &[]);
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
) -> Result<(), Reason> {
    let result = if seats.players.by_conn(conn).is_some() {
        match drop_conn(world, seats, conn) {
            Ok(true) => {
                print_host(world, &[]);
                Ok(())
            }
            Ok(false) => Ok(()),
            Err(code) => Err(code),
        }
    } else if seats.gm.as_ref().is_some_and(|gm| gm.conn == conn) {
        seats.gm = None;
        Ok(())
    } else {
        Ok(())
    };
    let _ = ack.send(());
    result
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
