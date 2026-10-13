use std::io::{self, Write};

use spookery_kernel::commit::commit_presence;
use spookery_kernel::model::{ActorLive, Presence};
use spookery_kernel::perceive::perceive;
use spookery_kernel::world::World;
use spookery_kernel::{idle_digest, DigestViewer, Reason};

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

pub fn seat_player(shard: &mut World, actor_id: &str) -> Result<String, Reason> {
    if !is_live_pc(shard, actor_id) {
        return Err(Reason::Schema);
    }
    let mut ids = shard.now().presence.actors_present.clone();
    if ids.len() >= MAX_PLAYER_SEATS {
        return Err(Reason::SessionFull);
    }
    if ids.iter().any(|id| id == actor_id) {
        return Err(Reason::SeatTaken);
    }
    ids.push(actor_id.to_string());
    commit_held(shard, &ids)?;
    // Seating always changes presence, so `state_rev` has just moved and
    // never repeats. A seat count would reuse an id after a drop.
    Ok(format!("s{}", shard.now().state_rev))
}

pub fn release_player_seat(
    shard: &mut World,
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
    let mut ids = shard.now().presence.actors_present.clone();
    ids.retain(|id| id != actor_id);
    commit_held(shard, &ids)
}

fn commit_held(shard: &mut World, ids: &[String]) -> Result<bool, Reason> {
    commit_presence(
        shard,
        Presence {
            humans: u32::try_from(ids.len()).map_err(|_| Reason::Schema)?,
            actors_present: ids.to_vec(),
        },
    )
}

fn is_live_pc(shard: &World, actor_id: &str) -> bool {
    shard
        .now()
        .actors
        .get(actor_id)
        .is_some_and(ActorLive::is_pc)
}

pub(super) fn clear_crashed(shard: &mut World) -> Result<bool, Reason> {
    let presence = &shard.now().presence;
    if presence.humans == 0 && presence.actors_present.is_empty() {
        return Ok(false);
    }
    commit_presence(
        shard,
        Presence {
            humans: 0,
            actors_present: Vec::new(),
        },
    )
}

pub(super) fn run_world_loop(
    mut shard: World,
    rx: std::sync::mpsc::Receiver<Msg>,
    restart: &tokio::sync::watch::Sender<u64>,
) -> ! {
    let mut seats = Seats::default();
    let mut next_session = 1u64;
    debug_seats(&seats, &shard);
    loop {
        let msg = match rx.recv() {
            Ok(msg) => msg,
            Err(_) => {
                eprintln!("{}", Reason::NotInSpec);
                std::process::exit(1);
            }
        };
        if let Err(code) = handle(&mut shard, &mut seats, &mut next_session, restart, msg) {
            eprintln!("{code}");
            std::process::exit(1);
        }
        debug_seats(&seats, &shard);
    }
}

pub(super) fn print_host(shard: &World) {
    if let Ok(text) = idle_digest(shard, DigestViewer::Host) {
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
        ack: Ack,
    },
    Tool {
        name: String,
        arguments: Option<serde_json::Map<String, serde_json::Value>>,
        reply: tokio::sync::oneshot::Sender<ToolReply>,
    },
    Wake,
    Stdin(String),
    StdinEof,
}

/// The shard thread answers a fault once the seat is released.
/// An async caller awaits `Task`. A caller that must wait from a plain `Drop`
/// uses `Thread` and blocks on it with a timeout.
pub(super) enum Ack {
    Task(tokio::sync::oneshot::Sender<()>),
    Thread(std::sync::mpsc::SyncSender<()>),
}

impl Ack {
    fn send(self) {
        match self {
            Ack::Task(tx) => {
                let _ = tx.send(());
            }
            Ack::Thread(tx) => {
                let _ = tx.try_send(());
            }
        }
    }
}

pub(super) struct ToolReply {
    pub value: serde_json::Value,
    pub after: Vec<Vec<u8>>,
}

pub(super) enum HelloReply {
    Refused(String),
    Player,
    Gm,
}

pub(super) enum PlayerReply {
    Done,
    Hangup,
}

fn handle(
    shard: &mut World,
    seats: &mut Seats,
    next_session: &mut u64,
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
            let answer = on_hello(shard, seats, next_session, conn, &line, out, cancel)?;
            if reply.send(answer).is_err() && seats.gm.as_ref().is_some_and(|gm| gm.conn == conn) {
                seats.gm = None;
            }
        }
        Msg::Player { conn, line, reply } => {
            let answer = on_player(shard, seats, restart, conn, &line)?;
            let _ = reply.send(answer);
        }
        Msg::Fault { conn, end, ack } => on_fault(shard, seats, conn, end, ack)?,
        Msg::Tool {
            name,
            arguments,
            reply,
        } => {
            let mut done = super::gm_tools::call_gm_tool(shard, &name, arguments);
            let notices = if done.ticked {
                let notices = shard.take_rolls();
                frames::write_checks(&mut done.value, &notices);
                notices
            } else {
                Vec::new()
            };
            if done.ticked {
                refresh_players(shard, seats, &notices)?;
                print_host(shard);
            }
            if let Some(tape) = &done.tape {
                println!("{tape}");
                let _ = io::stdout().flush();
            }
            if let Some(frame) = done.gm_event {
                send_gm(seats, frame);
            }
            if let Some((recipients, line)) = &done.player_line {
                send_recipients(shard, seats, recipients, line.clone())?;
            }
            let mut after = done.after;
            after.extend(player_moves::gm_check_frames(&notices));
            if done.advanced {
                bump_sleep(restart);
            }
            let _ = reply.send(ToolReply {
                value: done.value,
                after,
            });
        }
        Msg::Wake => {
            if idle_clock::on_scheduled_wake(shard, !seats.players.is_empty(), seats.gm.is_some())?
            {
                bump_sleep(restart);
            }
        }
        Msg::Stdin(line) if line == "stop" => shutdown(shard, seats),
        Msg::Stdin(_) => eprintln!("{}", Reason::NotInSpec),
        Msg::StdinEof => shutdown(shard, seats),
    }
    Ok(())
}

fn bump_sleep(restart: &tokio::sync::watch::Sender<u64>) {
    let next = restart.borrow().wrapping_add(1);
    let _ = restart.send(next);
}

fn refresh_players(
    shard: &mut World,
    seats: &mut Seats,
    notices: &[spookery_kernel::rules::RollReport],
) -> Result<(), Reason> {
    let mut failed = Vec::new();
    for player in seats.players.slots.iter().flatten() {
        let bytes = match player_refresh(shard, &player.actor_id) {
            Ok(text) => {
                let mut out = player_moves::player_notice_lines(notices, &player.actor_id);
                out.push_str(&text);
                out.into_bytes()
            }
            Err(code) => refused_line(code).into_bytes(),
        };
        if !super::limits::try_frame(&player.out, bytes) {
            failed.push(player.conn);
        }
    }
    for conn in failed {
        if drop_conn(shard, seats, conn, SocketEnd::Write)? {
            print_host(shard);
        }
    }
    Ok(())
}

fn shutdown(shard: &mut World, seats: &mut Seats) -> ! {
    for conn in seats.players.conns() {
        match drop_conn(shard, seats, conn, SocketEnd::Eof) {
            Ok(true) => print_host(shard),
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
    shard: &mut World,
    seats: &mut Seats,
    next_session: &mut u64,
    conn: u64,
    line: &str,
    out: OutTx,
    cancel: CancelTx,
) -> Result<HelloReply, Reason> {
    if line == "gm" {
        if seats.gm.is_some() {
            return Ok(HelloReply::Refused(refused_line(Reason::SessionFull)));
        }
        seats.gm = Some(GmSeat { conn, out, cancel });
        return Ok(HelloReply::Gm);
    }
    if let Some(id) = player_hello(line) {
        if seats.players.is_full() {
            return Ok(HelloReply::Refused(refused_line(Reason::SessionFull)));
        }
        if seats.players.holds(id) {
            return Ok(HelloReply::Refused(refused_line(Reason::SeatTaken)));
        }
        if !is_live_pc(shard, id) {
            return Ok(HelloReply::Refused(refused_line(Reason::Schema)));
        }
        let session_id = format!("s{next_session}");
        *next_session = next_session.checked_add(1).ok_or(Reason::Schema)?;
        let mut ids = seats.players.actor_ids();
        ids.push(id.to_string());
        if let Err(code) = commit_held(shard, &ids) {
            return Ok(HelloReply::Refused(refused_line(code)));
        }
        let bytes = match welcome(shard, id) {
            Ok(bytes) => bytes,
            Err(code) => {
                if release_player_seat(shard, &session_id, id, SocketEnd::Close)? {
                    print_host(shard);
                }
                return Ok(HelloReply::Refused(refused_line(code)));
            }
        };
        if !super::limits::try_frame(&out, bytes) {
            if release_player_seat(shard, &session_id, id, SocketEnd::Write)? {
                print_host(shard);
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
            if release_player_seat(shard, &session_id, id, SocketEnd::Close)? {
                print_host(shard);
            }
            return Ok(HelloReply::Refused(refused_line(code)));
        }
        print_host(shard);
        return Ok(HelloReply::Player);
    }
    Ok(HelloReply::Refused(refused_line(Reason::Schema)))
}

fn player_hello(line: &str) -> Option<&str> {
    crate::protocol::read_player_hello(line)
}

fn welcome(shard: &World, actor_id: &str) -> Result<Vec<u8>, Reason> {
    let mut out = String::new();
    for line in super::speech::player_scrollback(shard.root(), actor_id)? {
        out.push_str(&line);
    }
    out.push_str(&player_refresh(shard, actor_id)?);
    Ok(out.into_bytes())
}

fn player_refresh(shard: &World, actor_id: &str) -> Result<String, Reason> {
    let digest = idle_digest(shard, DigestViewer::Player(actor_id.to_string()))?;
    let view = perceive(shard, &format!("player:{actor_id}"))?;
    let mut out = frames::player_resume_line(&digest);
    out.push_str(&frames::player_perceive_line(&view));
    Ok(out)
}

fn on_player(
    shard: &mut World,
    seats: &mut Seats,
    restart: &tokio::sync::watch::Sender<u64>,
    conn: u64,
    line: &str,
) -> Result<PlayerReply, Reason> {
    if is_close(line) {
        if drop_conn(shard, seats, conn, SocketEnd::Close)? {
            print_host(shard);
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
    match input::parse_player_input(line, &actor_id, &shard.canon().moves) {
        Ok(input::PlayerLine::Speech) => match speak(shard, &actor_id, line) {
            Ok(spoken) => {
                println!("{}", spoken.tape);
                let _ = io::stdout().flush();
                send_gm(seats, spoken.gm_event);
                send_recipients(
                    shard,
                    seats,
                    &spoken.recipients,
                    spoken.player_event.into_bytes(),
                )?;
                Ok(PlayerReply::Done)
            }
            Err(code) => {
                send_player(shard, seats, conn, refused_line(code).into_bytes())?;
                Ok(PlayerReply::Done)
            }
        },
        Ok(input::PlayerLine::Turn) => on_mechanical(shard, seats, restart, conn, &actor_id, line),
        Err(code) => {
            send_player(shard, seats, conn, refused_line(code).into_bytes())?;
            Ok(PlayerReply::Done)
        }
    }
}

fn on_mechanical(
    shard: &mut World,
    seats: &mut Seats,
    restart: &tokio::sync::watch::Sender<u64>,
    conn: u64,
    actor_id: &str,
    line: &str,
) -> Result<PlayerReply, Reason> {
    let gm_seated = seats.gm.is_some();
    match player_moves::handle_player_move(shard, actor_id, line, gm_seated) {
        Ok(player_moves::MoveOutcome::Queued(note)) => {
            send_gm(seats, note.gm_frame());
            send_player(shard, seats, conn, note.player_line().into_bytes())?;
        }
        Ok(player_moves::MoveOutcome::Committed { advanced, notices }) => {
            if advanced {
                bump_sleep(restart);
            }
            print_host(shard);
            refresh_players(shard, seats, &notices)?;
        }
        Err(code) => send_player(shard, seats, conn, refused_line(code).into_bytes())?,
    }
    Ok(PlayerReply::Done)
}

/// A full or closed queue drops that seat. The shard thread does not wait.
fn send_player(
    shard: &mut World,
    seats: &mut Seats,
    conn: u64,
    bytes: Vec<u8>,
) -> Result<(), Reason> {
    let failed = match seats.players.by_conn(conn) {
        Some(player) => !super::limits::try_frame(&player.out, bytes),
        None => false,
    };
    if failed && drop_conn(shard, seats, conn, SocketEnd::Write)? {
        print_host(shard);
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
    shard: &mut World,
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
        if drop_conn(shard, seats, conn, SocketEnd::Write)? {
            print_host(shard);
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
    shard: &mut World,
    seats: &mut Seats,
    conn: u64,
    end: SocketEnd,
) -> Result<bool, Reason> {
    let Some(player) = seats.players.take_conn(conn) else {
        return Ok(false);
    };
    release_player_seat(shard, &player.session_id, &player.actor_id, end)
}

fn on_fault(
    shard: &mut World,
    seats: &mut Seats,
    conn: u64,
    end: SocketEnd,
    ack: Ack,
) -> Result<(), Reason> {
    let result = if seats.players.by_conn(conn).is_some() {
        match drop_conn(shard, seats, conn, end) {
            Ok(true) => {
                print_host(shard);
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
    ack.send();
    result
}

fn refused_line(code: Reason) -> String {
    frames::player_refused_line(code.token())
}

fn debug_seats(seats: &Seats, shard: &World) {
    let mut held = seats.players.actor_ids();
    held.sort();
    let presence = &shard.now().presence;
    debug_assert_eq!(presence.humans as usize, held.len());
    let mut present = presence.actors_present.clone();
    present.sort();
    debug_assert_eq!(present, held);
}
