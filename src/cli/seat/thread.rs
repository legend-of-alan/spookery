use std::io::{self, Write};

use spookery_kernel::model::{ActorLive, Presence};
use spookery_kernel::perceive::perceive;
use spookery_kernel::shard::Shard;
use spookery_kernel::{idle_digest, DigestViewer, Reason};
use spookery_tick::commit_presence;

use super::protocol;
use super::speech::speak;
use super::turn;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SocketEnd {
    Eof,
    Read,
    Write,
    Keepalive,
    Close,
}

pub fn seat_player(shard: &mut Shard, actor_id: &str) -> Result<String, Reason> {
    hold_player(shard, actor_id)?;
    Ok("s1".to_string())
}

pub fn release_player_seat(
    shard: &mut Shard,
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
    let mut presence = shard.now().presence.clone();
    presence.humans = 0;
    presence.actors_present.retain(|id| id != actor_id);
    commit_presence(shard, presence)
}

pub(super) fn clear_crashed(shard: &mut Shard) -> Result<bool, Reason> {
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

pub(super) fn serve(
    mut shard: Shard,
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
        if let Err(code) = on_msg(&mut shard, &mut seats, &mut next_session, restart, msg) {
            eprintln!("{code}");
            std::process::exit(1);
        }
        debug_seats(&seats, &shard);
    }
}

pub(super) fn print_host(shard: &Shard) {
    if let Ok(text) = idle_digest(shard, DigestViewer::Host) {
        if !text.is_empty() {
            print!("{text}");
            let _ = io::stdout().flush();
        }
    }
}

struct Seats {
    player: Option<HeldPlayer>,
    gm: Option<GmSeat>,
}

impl Default for Seats {
    fn default() -> Self {
        Self {
            player: None,
            gm: None,
        }
    }
}

struct HeldPlayer {
    conn: u64,
    actor_id: String,
    session_id: String,
    out: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
}

struct GmSeat {
    conn: u64,
    out: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
}

pub(super) enum Msg {
    Hello {
        conn: u64,
        line: String,
        out: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
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
        name: String,
        arguments: Option<serde_json::Map<String, serde_json::Value>>,
        reply: tokio::sync::oneshot::Sender<ToolReply>,
    },
    Wake,
    Stdin(String),
    StdinEof,
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

fn hold_player(shard: &mut Shard, actor_id: &str) -> Result<(), Reason> {
    if !shard
        .now()
        .actors
        .get(actor_id)
        .is_some_and(ActorLive::is_pc)
    {
        return Err(Reason::Schema);
    }
    let mut presence = shard.now().presence.clone();
    presence.humans = 1;
    if !presence.actors_present.iter().any(|id| id == actor_id) {
        presence.actors_present.push(actor_id.to_string());
    }
    commit_presence(shard, presence)?;
    Ok(())
}

fn on_msg(
    shard: &mut Shard,
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
            reply,
        } => {
            let answer = on_hello(shard, seats, next_session, conn, &line, out)?;
            let _ = reply.send(answer);
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
            let done = super::tools::gm_tool(shard, &name, arguments);
            let notices = if done.ticked {
                shard.take_rolls()
            } else {
                Vec::new()
            };
            if done.ticked {
                refresh_player(shard, seats, &notices);
                print_host(shard);
            }
            if let Some(tape) = &done.tape {
                println!("{tape}");
                let _ = io::stdout().flush();
            }
            if let Some(frame) = done.gm_event {
                if let Some(gm) = &seats.gm {
                    let _ = gm.out.send(frame);
                }
            }
            if let Some((recipients, line)) = &done.player_line {
                if let Some(player) = &seats.player {
                    if recipients.iter().any(|id| id == &player.actor_id) {
                        let _ = player.out.send(line.clone());
                    }
                }
            }
            let mut after = done.after;
            after.extend(turn::gm_check_frames(&notices));
            if done.advanced {
                bump_sleep(restart);
            }
            let _ = reply.send(ToolReply {
                value: done.value,
                after,
            });
        }
        Msg::Wake => {
            if turn::on_scheduled_wake(shard, seats.player.is_some(), seats.gm.is_some())? {
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

fn refresh_player(shard: &Shard, seats: &Seats, notices: &[spookery_kernel::check::CheckNotice]) {
    let Some(player) = &seats.player else {
        return;
    };
    let mut out = turn::player_notice_lines(notices, &player.actor_id);
    if let Ok(text) = player_refresh(shard, &player.actor_id) {
        out.push_str(&text);
        let _ = player.out.send(out.into_bytes());
    }
}

fn shutdown(shard: &mut Shard, seats: &mut Seats) -> ! {
    if seats.player.is_some() {
        match drop_player(shard, seats, SocketEnd::Eof) {
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
    shard: &mut Shard,
    seats: &mut Seats,
    next_session: &mut u64,
    conn: u64,
    line: &str,
    out: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
) -> Result<HelloReply, Reason> {
    if line == "gm" {
        if seats.gm.is_some() {
            return Ok(HelloReply::Refused(refused_line(Reason::SessionFull)));
        }
        seats.gm = Some(GmSeat { conn, out });
        return Ok(HelloReply::Gm);
    }
    if let Some(id) = player_hello(line) {
        if seats.player.is_some() {
            return Ok(HelloReply::Refused(refused_line(Reason::SessionFull)));
        }
        if !shard
            .now()
            .actors
            .get(id)
            .is_some_and(ActorLive::is_pc)
        {
            return Ok(HelloReply::Refused(refused_line(Reason::Schema)));
        }
        let session_id = format!("s{next_session}");
        *next_session = next_session.checked_add(1).ok_or(Reason::Schema)?;
        if let Err(code) = hold_player(shard, id) {
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
        let _ = out.send(bytes);
        seats.player = Some(HeldPlayer {
            conn,
            actor_id: id.to_string(),
            session_id,
            out,
        });
        print_host(shard);
        return Ok(HelloReply::Player);
    }
    Ok(HelloReply::Refused(refused_line(Reason::Schema)))
}

fn player_hello(line: &str) -> Option<&str> {
    let id = line.strip_prefix("player:")?;
    if id.is_empty() || id.chars().any(char::is_whitespace) {
        return None;
    }
    Some(id)
}

fn welcome(shard: &Shard, actor_id: &str) -> Result<Vec<u8>, Reason> {
    let mut out = String::new();
    for line in super::speech::player_scrollback(shard.root(), actor_id)? {
        out.push_str(&line);
    }
    out.push_str(&player_refresh(shard, actor_id)?);
    Ok(out.into_bytes())
}

fn player_refresh(shard: &Shard, actor_id: &str) -> Result<String, Reason> {
    let digest = idle_digest(shard, DigestViewer::Player(actor_id.to_string()))?;
    let view = perceive(shard, &format!("player:{actor_id}"))?;
    let mut out = protocol::player_resume_line(&digest);
    out.push_str(&protocol::player_perceive_line(&view));
    Ok(out)
}

fn on_player(
    shard: &mut Shard,
    seats: &mut Seats,
    restart: &tokio::sync::watch::Sender<u64>,
    conn: u64,
    line: &str,
) -> Result<PlayerReply, Reason> {
    if is_close(line) {
        if seats
            .player
            .as_ref()
            .is_some_and(|player| player.conn == conn)
        {
            if drop_player(shard, seats, SocketEnd::Close)? {
                print_host(shard);
            }
        }
        return Ok(PlayerReply::Hangup);
    }
    let Some(actor_id) = seats
        .player
        .as_ref()
        .filter(|player| player.conn == conn)
        .map(|player| player.actor_id.clone())
    else {
        return Ok(PlayerReply::Hangup);
    };
    match turn::classify_player_line(line, &actor_id, &shard.canon().moves) {
        Ok(turn::PlayerLine::Speech) => match speak(shard, &actor_id, line) {
            Ok(spoken) => {
                println!("{}", spoken.tape);
                let _ = io::stdout().flush();
                if let Some(gm) = &seats.gm {
                    let _ = gm.out.send(spoken.gm_event);
                }
                send_player(seats, conn, spoken.player_event.into_bytes());
                Ok(PlayerReply::Done)
            }
            Err(code) => {
                send_player(seats, conn, refused_line(code).into_bytes());
                Ok(PlayerReply::Done)
            }
        },
        Ok(turn::PlayerLine::Turn) => on_mechanical(shard, seats, restart, conn, &actor_id, line),
        Err(code) => {
            send_player(seats, conn, refused_line(code).into_bytes());
            Ok(PlayerReply::Done)
        }
    }
}

fn on_mechanical(
    shard: &mut Shard,
    seats: &mut Seats,
    restart: &tokio::sync::watch::Sender<u64>,
    conn: u64,
    actor_id: &str,
    line: &str,
) -> Result<PlayerReply, Reason> {
    let gm_seated = seats.gm.is_some();
    match turn::turn(shard, actor_id, line, gm_seated) {
        Ok(turn::Turn::Queued(note)) => {
            if let Some(gm) = &seats.gm {
                let _ = gm.out.send(note.gm_frame());
            }
            send_player(seats, conn, note.player_line().into_bytes());
        }
        Ok(turn::Turn::Committed { advanced, notices }) => {
            if advanced {
                bump_sleep(restart);
            }
            print_host(shard);
            match player_refresh(shard, actor_id) {
                Ok(refresh) => {
                    let mut bytes = turn::player_notice_lines(&notices, actor_id);
                    bytes.push_str(&refresh);
                    send_player(seats, conn, bytes.into_bytes());
                }
                Err(code) => send_player(seats, conn, refused_line(code).into_bytes()),
            }
        }
        Err(code) => send_player(seats, conn, refused_line(code).into_bytes()),
    }
    Ok(PlayerReply::Done)
}

fn send_player(seats: &Seats, conn: u64, bytes: Vec<u8>) {
    if let Some(player) = seats.player.as_ref().filter(|player| player.conn == conn) {
        let _ = player.out.send(bytes);
    }
}

fn is_close(line: &str) -> bool {
    match line.split_once(' ') {
        Some((token, rest)) => token == "close" && rest.is_empty(),
        None => line == "close",
    }
}

fn drop_player(shard: &mut Shard, seats: &mut Seats, end: SocketEnd) -> Result<bool, Reason> {
    let Some(player) = seats.player.take() else {
        return Ok(false);
    };
    release_player_seat(shard, &player.session_id, &player.actor_id, end)
}

fn on_fault(
    shard: &mut Shard,
    seats: &mut Seats,
    conn: u64,
    end: SocketEnd,
    ack: tokio::sync::oneshot::Sender<()>,
) -> Result<(), Reason> {
    let result = if seats
        .player
        .as_ref()
        .is_some_and(|player| player.conn == conn)
    {
        match drop_player(shard, seats, end) {
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
    let _ = ack.send(());
    result
}

fn refused_line(code: Reason) -> String {
    protocol::player_refused_line(code.token())
}

fn debug_seats(seats: &Seats, shard: &Shard) {
    debug_assert_eq!(seats.player.is_some(), shard.now().presence.humans == 1);
    if let Some(player) = &seats.player {
        debug_assert!(shard
            .now()
            .presence
            .actors_present
            .iter()
            .any(|id| id == &player.actor_id));
    } else {
        let start = shard.canon().start_pc.as_str();
        debug_assert!(
            shard.now().presence.humans != 1
                || shard
                    .now()
                    .presence
                    .actors_present
                    .iter()
                    .all(|id| id != start)
        );
    }
}
