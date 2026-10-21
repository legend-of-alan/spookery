//! The table loop: seat players, read lines, and answer the GM.

use std::io::{self, Write};

use spookery_kernel::api::model::{ActorLive, Presence};
use spookery_kernel::api::perceive::perceive;
use spookery_kernel::api::world::World;
use spookery_kernel::api::{host_summary, player_digest, Reason};
use spookery_round::RoundOutput;

use super::frames;
use super::idle_clock;
use super::input;
use super::limits::{CancelTx, OutTx};
use super::player_moves;
use super::speech::speak;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SocketEnd {
    Eof,
    Read,
    Write,
    Keepalive,
    Close,
}

/// Engine cap. A pack may define more PCs than this.
const MAX_PLAYER_SEATS: usize = 3;

pub fn seat_player(world: &mut World, actor_id: &str) -> Result<String, Reason> {
    if !is_live_pc(world, actor_id) {
        return Err(Reason::Schema);
    }
    let mut ids = world.now().presence.actors_present.clone();
    if ids.len() >= MAX_PLAYER_SEATS {
        return Err(Reason::SessionFull);
    }
    if ids.iter().any(|id| id == actor_id) {
        return Err(Reason::SeatTaken);
    }
    ids.push(actor_id.to_string());
    commit_held(world, &ids)?;
    world.mint_session()
}

pub fn release_player_seat(
    world: &mut World,
    session_id: &str,
    actor_id: &str,
    end: SocketEnd,
) -> Result<bool, Reason> {
    match end {
        SocketEnd::Eof
        | SocketEnd::Read
        | SocketEnd::Write
        | SocketEnd::Keepalive
        | SocketEnd::Close => {}
    }
    if session_id.is_empty() {
        return Err(Reason::Schema);
    }
    let mut ids = world.now().presence.actors_present.clone();
    ids.retain(|id| id != actor_id);
    commit_held(world, &ids)
}

fn commit_held(world: &mut World, ids: &[String]) -> Result<bool, Reason> {
    Ok(world.set_presence(Presence {
        humans: u32::try_from(ids.len()).map_err(|_| Reason::Schema)?,
        actors_present: ids.to_vec(),
    }))
}

fn is_live_pc(world: &World, actor_id: &str) -> bool {
    world
        .now()
        .actors
        .get(actor_id)
        .is_some_and(ActorLive::is_pc)
}

pub(super) fn clear_crashed(world: &mut World) -> Result<bool, Reason> {
    let presence = &world.now().presence;
    if presence.humans == 0 && presence.actors_present.is_empty() {
        return Ok(false);
    }
    Ok(world.set_presence(Presence {
        humans: 0,
        actors_present: Vec::new(),
    }))
}

pub(super) fn run_world_loop(
    mut world: World,
    rx: std::sync::mpsc::Receiver<Msg>,
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

pub(super) fn print_host(world: &World) {
    if let Ok(text) = host_summary(world) {
        if !text.is_empty() {
            print!("{text}");
            let _ = io::stdout().flush();
        }
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

    fn is_full(&self) -> bool {
        self.len() == MAX_PLAYER_SEATS
    }

    fn holds(&self, actor_id: &str) -> bool {
        self.slots
            .iter()
            .flatten()
            .any(|player| player.actor_id == actor_id)
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

pub(super) enum Msg {
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
        end: SocketEnd,
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
        reply: tokio::sync::oneshot::Sender<Result<(), String>>,
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
    msg: Msg,
) -> Result<(), Reason> {
    match msg {
        Msg::Hello {
            conn,
            line,
            out,
            cancel,
            reply,
        } => {
            let answer = on_hello(world, seats, conn, &line, out, cancel)?;
            let _ = reply.send(answer);
        }
        Msg::Claim {
            conn,
            out,
            cancel,
            reply,
        } => {
            let answer = seat_gm(seats, conn, out, cancel);
            if reply.send(answer).is_err() && seats.gm.as_ref().is_some_and(|gm| gm.conn == conn) {
                seats.gm = None;
            }
        }
        Msg::Player { conn, line, reply } => {
            let answer = on_player(world, seats, restart, conn, &line)?;
            let _ = reply.send(answer);
        }
        Msg::Fault { conn, end, ack } => on_fault(world, seats, conn, end, ack)?,
        Msg::Tool {
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
                print_host(world);
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
            after.extend(player_moves::gm_check_frames(&reports));
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
        Msg::Wake => {
            if idle_clock::on_scheduled_wake(world, !seats.players.is_empty(), seats.gm.is_some())?
            {
                bump_sleep(restart);
            }
        }
        Msg::Stdin(line) if line == "stop" => shutdown(world, seats),
        Msg::Stdin(_) => eprintln!("{}", Reason::NotInSpec),
        Msg::StdinEof => shutdown(world, seats),
    }
    Ok(())
}

fn narrate_for(lines: &[spookery_kernel::api::visibility::Told], viewer: &str) -> Vec<String> {
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
                let mut out = player_moves::player_report_lines(&reports, &player.actor_id);
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
        if drop_conn(world, seats, conn, SocketEnd::Write)? {
            print_host(world);
        }
    }
    Ok(())
}

fn shutdown(world: &mut World, seats: &mut Seats) -> ! {
    for conn in seats.players.conns() {
        match drop_conn(world, seats, conn, SocketEnd::Eof) {
            Ok(true) => print_host(world),
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
    if let Some(id) = player_hello(line) {
        if seats.players.is_full() {
            return Ok(HelloReply::Refused(refused_line(Reason::SessionFull)));
        }
        if seats.players.holds(id) {
            return Ok(HelloReply::Refused(refused_line(Reason::SeatTaken)));
        }
        if !is_live_pc(world, id) {
            return Ok(HelloReply::Refused(refused_line(Reason::Schema)));
        }
        let mut ids = seats.players.actor_ids();
        ids.push(id.to_string());
        if let Err(code) = commit_held(world, &ids) {
            return Ok(HelloReply::Refused(refused_line(code)));
        }
        let session_id = match world.mint_session() {
            Ok(session_id) => session_id,
            Err(code) => return Ok(HelloReply::Refused(refused_line(code))),
        };
        let bytes = match welcome(world, id) {
            Ok(bytes) => bytes,
            Err(code) => {
                if release_player_seat(world, &session_id, id, SocketEnd::Close)? {
                    print_host(world);
                }
                return Ok(HelloReply::Refused(refused_line(code)));
            }
        };
        if !super::limits::try_frame(&out, bytes) {
            if release_player_seat(world, &session_id, id, SocketEnd::Write)? {
                print_host(world);
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
            if release_player_seat(world, &session_id, id, SocketEnd::Close)? {
                print_host(world);
            }
            return Ok(HelloReply::Refused(refused_line(code)));
        }
        print_host(world);
        return Ok(HelloReply::Player);
    }
    Ok(HelloReply::Refused(refused_line(Reason::Schema)))
}

fn seat_gm(seats: &mut Seats, conn: u64, out: OutTx, cancel: CancelTx) -> Result<(), String> {
    if seats.gm.is_some() {
        return Err(refused_line(Reason::SessionFull));
    }
    seats.gm = Some(GmSeat { conn, out, cancel });
    Ok(())
}

fn player_hello(line: &str) -> Option<&str> {
    crate::protocol::read_player_hello(line)
}

fn welcome(world: &World, actor_id: &str) -> Result<Vec<u8>, Reason> {
    let mut out = String::new();
    for line in super::speech::player_scrollback(world.root(), actor_id)? {
        out.push_str(&line);
    }
    out.push_str(&player_refresh(world, actor_id)?);
    Ok(out.into_bytes())
}

fn player_refresh(world: &World, actor_id: &str) -> Result<String, Reason> {
    let view = perceive(world, &format!("player:{actor_id}"))?;
    let mut out = frames::player_resume_line(&player_digest(&view)?);
    out.push_str(&frames::player_perceive_line(&view));
    Ok(out)
}

fn on_player(
    world: &mut World,
    seats: &mut Seats,
    restart: &tokio::sync::watch::Sender<u64>,
    conn: u64,
    line: &str,
) -> Result<PlayerReply, Reason> {
    if is_close(line) {
        if drop_conn(world, seats, conn, SocketEnd::Close)? {
            print_host(world);
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
    match input::parse_player_input(line, &actor_id, &world.canon().rules) {
        Ok(input::PlayerLine::Speech) => match speak(world, &actor_id, line) {
            Ok(spoken) => {
                println!("{}", spoken.tape);
                let _ = io::stdout().flush();
                send_gm(seats, spoken.gm_event);
                send_recipients(
                    world,
                    seats,
                    &spoken.recipients,
                    spoken.player_event.into_bytes(),
                )?;
                Ok(PlayerReply::Done)
            }
            Err(code) => {
                send_player(world, seats, conn, refused_line(code).into_bytes())?;
                Ok(PlayerReply::Done)
            }
        },
        Ok(input::PlayerLine::Help) => {
            let text = spookery_kernel::api::map::help_lines(world.canon(), world.now(), &actor_id);
            send_player(world, seats, conn, frames::player_lines(&text).into_bytes())?;
            Ok(PlayerReply::Done)
        }
        Ok(input::PlayerLine::Look) => {
            let text = spookery_kernel::api::map::look_lines(world.canon(), world.now(), &actor_id);
            send_player(world, seats, conn, frames::player_lines(&text).into_bytes())?;
            Ok(PlayerReply::Done)
        }
        Ok(input::PlayerLine::Map) => {
            let text =
                spookery_kernel::api::map::player_lines(world.canon(), world.now(), &actor_id);
            send_player(world, seats, conn, frames::player_lines(&text).into_bytes())?;
            Ok(PlayerReply::Done)
        }
        Ok(input::PlayerLine::Turn) => on_mechanical(world, seats, restart, conn, &actor_id, line),
        Err(code) => {
            send_player(world, seats, conn, refused_line(code).into_bytes())?;
            Ok(PlayerReply::Done)
        }
    }
}

fn on_mechanical(
    world: &mut World,
    seats: &mut Seats,
    restart: &tokio::sync::watch::Sender<u64>,
    conn: u64,
    actor_id: &str,
    line: &str,
) -> Result<PlayerReply, Reason> {
    let gm_seated = seats.gm.is_some();
    match player_moves::handle_player_move(world, actor_id, line, gm_seated) {
        Ok(player_moves::MoveOutcome::Queued(note)) => {
            send_gm(seats, note.gm_frame());
            send_player(world, seats, conn, note.player_line().into_bytes())?;
        }
        Ok(player_moves::MoveOutcome::Committed { advanced, outputs }) => {
            if advanced {
                bump_sleep(restart);
            }
            print_host(world);
            refresh_players(world, seats, &outputs)?;
        }
        Ok(player_moves::MoveOutcome::Busy {
            code,
            missing,
            outputs,
        }) => {
            send_player(
                world,
                seats,
                conn,
                frames::player_refused(code, missing.as_deref()).into_bytes(),
            )?;
            if !outputs.is_empty() {
                refresh_players(world, seats, &outputs)?;
                print_host(world);
            }
        }
        Err(miss) => send_player(
            world,
            seats,
            conn,
            frames::player_refused(miss.token(), miss.missing()).into_bytes(),
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
    if failed && drop_conn(world, seats, conn, SocketEnd::Write)? {
        print_host(world);
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
        if drop_conn(world, seats, conn, SocketEnd::Write)? {
            print_host(world);
        }
    }
    Ok(())
}

fn is_close(line: &str) -> bool {
    match line.split_once(' ') {
        Some((token, rest)) => token == "close" && rest.is_empty(),
        None => line == "close",
    }
}

fn drop_conn(
    world: &mut World,
    seats: &mut Seats,
    conn: u64,
    end: SocketEnd,
) -> Result<bool, Reason> {
    let Some(player) = seats.players.take_conn(conn) else {
        return Ok(false);
    };
    release_player_seat(world, &player.session_id, &player.actor_id, end)
}

fn on_fault(
    world: &mut World,
    seats: &mut Seats,
    conn: u64,
    end: SocketEnd,
    ack: tokio::sync::oneshot::Sender<()>,
) -> Result<(), Reason> {
    let result = if seats.players.by_conn(conn).is_some() {
        match drop_conn(world, seats, conn, end) {
            Ok(true) => {
                print_host(world);
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
