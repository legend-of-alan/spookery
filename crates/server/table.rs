//! The table, the one owner of the world and its seats: hellos, lines, tools, say and rounds.

use std::io::Write;
use std::sync::mpsc::{Receiver, Sender};

use serde_json::{Map, Value};
use spookery_engine::{
    emit_tape_record, Address, Presence, Reason, RecipientLine, RoundOutput, TapeRecord, World,
};
use tokio::sync::{oneshot, watch};

use super::frames;
use super::idle::{self, now_stamp};
use super::seat::{Conn, FrameSender, GmSeat, PlayerSeat, Seats};
use super::tools::call_gm_tool;
use super::Error;
use crate::protocol::{self, Detail, Refusal, Speaker};

/// The host's cap. A pack may define more PCs than this.
const MAX_PLAYER_SEATS: usize = 3;

/// Commit a queued round, then fast-forward while every seated PC is busy.
/// An empty queue only fast-forwards.
pub(crate) fn commit_then_fast_forward(
    world: &mut World,
    presence: &Presence,
) -> Result<Vec<RoundOutput>, Reason> {
    let wall = now_stamp();
    world.commit_then_fast_forward(presence, &wall)
}

/// Sent from the socket tasks, the agent door and the idle timer to the table thread.
pub(super) enum Request {
    Hello {
        conn: Conn,
        line: String,
        frames_tx: FrameSender,
        reply: oneshot::Sender<HelloReply>,
    },
    Line {
        conn: Conn,
        line: String,
        reply: oneshot::Sender<()>,
    },
    Hangup {
        conn: Conn,
        ack: oneshot::Sender<()>,
    },
    Tool {
        conn: Conn,
        name: String,
        arguments: Option<Map<String, Value>>,
        reply: oneshot::Sender<Value>,
    },
    /// The HTTP session takes the one GM seat. The game port does not.
    SeatGm {
        conn: Conn,
        frames_tx: FrameSender,
        reply: oneshot::Sender<Result<(), Refusal>>,
    },
    Wake,
    Stop,
}

/// Tells the table a connection hung up, and waits until the table has let go of its seat.
pub(super) async fn report_hangup(tx: &Sender<Request>, conn: Conn) {
    let (ack_tx, ack_rx) = oneshot::channel();
    if tx.send(Request::Hangup { conn, ack: ack_tx }).is_err() {
        return;
    }
    let _ = ack_rx.await;
}

/// Sent from the table back to a player's socket task, answering its hello.
pub(super) enum HelloReply {
    Refused(Refusal),
    Seated,
}

pub(crate) struct Table {
    world: World,
    seats: Seats,
    /// Each send restarts the idle timer.
    idle: watch::Sender<u64>,
    out: Box<dyn Write>,
    err: Box<dyn Write>,
}

impl Table {
    pub(crate) fn new(
        world: World,
        idle: watch::Sender<u64>,
        out: Box<dyn Write>,
        err: Box<dyn Write>,
    ) -> Self {
        Self {
            world,
            seats: Seats::default(),
            idle,
            out,
            err,
        }
    }

    /// Answer requests until `Stop`. A store fault ends the table.
    pub(crate) fn run(mut self, rx: Receiver<Request>) -> Result<(), Error> {
        loop {
            let request = rx.recv().map_err(|_| Error::Reason(Reason::NotInSpec))?;
            let mut handled = Ok(());
            match request {
                Request::Hello {
                    conn,
                    line,
                    frames_tx,
                    reply,
                } => {
                    let _ = reply.send(self.hello(conn, &line, frames_tx));
                }
                Request::SeatGm {
                    conn,
                    frames_tx,
                    reply,
                } => {
                    let seated = self.seat_gm(conn, frames_tx);
                    let took = seated.is_ok();
                    if reply.send(seated).is_err() && took {
                        self.release_gm();
                    }
                }
                Request::Line { conn, line, reply } => {
                    self.handle_line(conn, &line);
                    let _ = reply.send(());
                }
                Request::Hangup { conn, ack } => {
                    self.handle_hangup(conn);
                    let _ = ack.send(());
                }
                Request::Tool {
                    conn,
                    name,
                    arguments,
                    reply,
                } => {
                    let _ = reply.send(self.call_tool(conn, &name, arguments));
                }
                Request::Wake => handled = self.wake(),
                Request::Stop => {
                    self.stop();
                    return Ok(());
                }
            }
            if let Some(warning) = self.world.take_warning() {
                let _ = writeln!(self.err, "{warning}");
                let _ = self.err.flush();
            }
            handled.map_err(Error::Reason)?;
        }
    }

    fn hello(&mut self, conn: Conn, line: &str, frames_tx: FrameSender) -> HelloReply {
        let named = match line.parse() {
            _ if line.is_empty() => None,
            Ok(Speaker::Gm) => return HelloReply::Refused(Reason::NotInSpec.into()),
            Ok(Speaker::Player(id)) => Some(id),
            Ok(Speaker::Npc(_)) | Err(_) => return HelloReply::Refused(Reason::Schema.into()),
        };
        if self.seats.players.len() >= MAX_PLAYER_SEATS {
            return HelloReply::Refused(Reason::TableFull.into());
        }
        let Some(id) = named else {
            let free = self.world.free_pcs(&self.seats.presence());
            return HelloReply::Refused(Refusal {
                code: protocol::Code::Known(Reason::ChoosePc),
                detail: Some(Detail::Many(free)),
            });
        };
        if self
            .seats
            .players
            .iter()
            .any(|player| player.actor_id == id)
        {
            return HelloReply::Refused(Reason::SeatTaken.into());
        }
        if !self
            .world
            .state()
            .actors
            .get(&id)
            .is_some_and(|actor| actor.is_pc())
        {
            return HelloReply::Refused(Reason::Schema.into());
        }
        // Seat first, so the welcome view counts this player as present.
        self.seats.players.push(PlayerSeat {
            conn,
            actor_id: id.clone(),
            out: frames_tx,
        });
        let sent = welcome(&self.world, &self.seats.presence(), &id).and_then(|bytes| {
            let seated = self
                .seats
                .player(conn)
                .is_some_and(|player| player.out.send(bytes));
            if seated {
                Ok(())
            } else {
                Err(Reason::Schema)
            }
        });
        if let Err(code) = sent {
            self.drop_player(conn);
            return HelloReply::Refused(code.into());
        }
        self.write_digest(&[]);
        HelloReply::Seated
    }

    fn seat_gm(&mut self, conn: Conn, frames_tx: FrameSender) -> Result<(), Refusal> {
        if self.seats.gm.is_some() {
            return Err(Reason::TableFull.into());
        }
        self.seats.gm = Some(GmSeat {
            conn,
            out: frames_tx,
        });
        Ok(())
    }

    fn handle_line(&mut self, conn: Conn, line: &str) {
        let Some(actor_id) = self
            .seats
            .player(conn)
            .map(|player| player.actor_id.clone())
        else {
            return;
        };
        let presence = self.seats.presence();
        let reply = match parse_command(&self.world, &actor_id, line) {
            Ok(PlayerCommand::Close) => return self.drop_player(conn),
            Ok(PlayerCommand::Help) => {
                frames::text_line(&self.world.help_lines(&presence, &actor_id))
            }
            Ok(PlayerCommand::Look) => {
                frames::text_line(&self.world.look_lines(&presence, &actor_id))
            }
            Ok(PlayerCommand::Map) => {
                frames::text_line(&self.world.map_lines(&presence, &actor_id))
            }
            Ok(PlayerCommand::Say(heard, text)) => {
                match self.world.say_as_player(&presence, &actor_id, heard, text) {
                    Ok(said) => return self.deliver_say(&said),
                    Err(code) => frames::refused_line(code.into()),
                }
            }
            Ok(PlayerCommand::Move(verb, target)) => {
                return self.play_move(conn, &actor_id, verb, target);
            }
            Err(refusal) => frames::refused_line(refusal),
        };
        self.send_player(conn, reply.into_bytes());
    }

    /// With a GM seated, a line waits for `commit_round`. Without one it commits its
    /// own round, and a busy PC's line plays the rounds its action still needs.
    fn play_move(&mut self, conn: Conn, actor_id: &str, verb: &str, target: Option<&str>) {
        let gm_seated = self.seats.gm.is_some();
        let presence = self.seats.presence();
        let busy = match self
            .world
            .queue_player_move(&presence, actor_id, verb, target)
        {
            Ok(queued) if gm_seated => {
                self.send_gm(frames::queued_event(&queued));
                return self.send_player(conn, frames::queued_line(&queued.id).into_bytes());
            }
            Ok(_) => None,
            Err(refusal) if !gm_seated && refusal.reason() == Reason::Busy => Some(refusal),
            Err(refusal) => {
                let line = frames::refused_move_line(refusal.reason(), refusal.missing());
                return self.send_player(conn, line.into_bytes());
            }
        };
        let outputs = match commit_then_fast_forward(&mut self.world, &presence) {
            Ok(outputs) => outputs,
            Err(code) => {
                return self.send_player(conn, frames::refused_line(code.into()).into_bytes())
            }
        };
        if let Some(refusal) = &busy {
            let line = frames::refused_move_line(refusal.reason(), refusal.missing());
            self.send_player(conn, line.into_bytes());
        }
        self.after_rounds(&outputs);
    }

    fn call_tool(
        &mut self,
        conn: Conn,
        name: &str,
        arguments: Option<Map<String, Value>>,
    ) -> Value {
        if self.seats.gm.as_ref().is_none_or(|gm| gm.conn != conn) {
            return frames::refused_value(Reason::NotInSpec);
        }
        let presence = self.seats.presence();
        let reply = call_gm_tool(&mut self.world, &presence, name, arguments);
        if let Some(code) = &reply.refused {
            let _ = writeln!(self.out, "refused {name} {}", code.as_str());
            let _ = self.out.flush();
        }
        self.after_rounds(&reply.outputs);
        if let Some(said) = &reply.said {
            self.deliver_say(said);
        }
        for output in &reply.outputs {
            for frame in frames::check_events(&output.reports) {
                self.send_gm(frame);
            }
        }
        // Free the seat before the client can read this result.
        if reply.release_seat {
            self.release_gm();
        }
        reply.value
    }

    fn handle_hangup(&mut self, conn: Conn) {
        self.drop_player(conn);
        if self.seats.gm.as_ref().is_some_and(|gm| gm.conn == conn) {
            self.release_gm();
        }
    }

    /// Frees the GM seat and says so on the host log, so the operator sees the seat is open.
    fn release_gm(&mut self) {
        self.seats.release_gm();
        let _ = writeln!(self.out, "gm seat free");
        let _ = self.out.flush();
    }

    fn wake(&mut self) -> Result<(), Reason> {
        // A seated table plays its own rounds.
        if !self.seats.players.is_empty() || self.seats.gm.is_some() {
            return Ok(());
        }
        if idle::catch_up_if_due(&mut self.world)? {
            self.restart_idle_timer();
        }
        Ok(())
    }

    fn stop(&mut self) {
        let conns: Vec<Conn> = self
            .seats
            .players
            .iter()
            .map(|player| player.conn)
            .collect();
        for conn in conns {
            self.drop_player(conn);
        }
        self.seats.release_gm();
    }

    /// The one order after rounds: refresh every player, print the digest, then
    /// restart the idle timer.
    fn after_rounds(&mut self, outputs: &[RoundOutput]) {
        if outputs.is_empty() {
            return;
        }
        let presence = self.seats.presence();
        let mut failed = Vec::new();
        for player in &self.seats.players {
            let bytes = match refresh_lines(&self.world, &presence, &player.actor_id) {
                Ok(text) => {
                    let mut out: String = outputs
                        .iter()
                        .map(|output| frames::check_lines(&output.reports, &player.actor_id))
                        .collect();
                    out.push_str(&text);
                    for output in outputs {
                        for line in narrate_lines_for(&output.lines, &player.actor_id) {
                            out.push_str(&frames::text_line(&line));
                        }
                        for item in &output.seen {
                            if item.viewer == player.actor_id && !item.line.is_empty() {
                                out.push_str(&frames::text_line(&item.line));
                            }
                        }
                    }
                    out.into_bytes()
                }
                Err(code) => frames::refused_line(code.into()).into_bytes(),
            };
            if !player.out.send(bytes) {
                failed.push(player.conn);
            }
        }
        for conn in failed {
            self.drop_player(conn);
        }
        self.write_digest(outputs);
        self.restart_idle_timer();
    }

    fn restart_idle_timer(&self) {
        self.idle
            .send_modify(|count| *count = count.wrapping_add(1));
    }

    /// The one say path: the host's tape line, the GM's `say` event, and the
    /// recipients' `say` lines.
    fn deliver_say(&mut self, said: &TapeRecord) {
        let _ = writeln!(self.out, "{}", emit_tape_record(said));
        let _ = self.out.flush();
        self.send_gm(frames::say_event(said));
        let line = frames::say_line(said, false).into_bytes();
        let mut failed = Vec::new();
        for player in &self.seats.players {
            if said.recipients.iter().any(|id| id == &player.actor_id)
                && !player.out.send(line.clone())
            {
                failed.push(player.conn);
            }
        }
        for conn in failed {
            self.drop_player(conn);
        }
    }

    /// A full or closed queue drops that seat. The table does not wait.
    fn send_player(&mut self, conn: Conn, bytes: Vec<u8>) {
        if self
            .seats
            .player(conn)
            .is_some_and(|player| !player.out.send(bytes))
        {
            self.drop_player(conn);
        }
    }

    fn send_gm(&mut self, frame: Vec<u8>) {
        if self.seats.gm.as_ref().is_some_and(|gm| !gm.out.send(frame)) {
            self.release_gm();
        }
    }

    /// Drops a seated player and tells the host. Dropping the seat cancels its socket.
    fn drop_player(&mut self, conn: Conn) {
        if self.seats.take_player(conn).is_some() {
            self.write_digest(&[]);
        }
    }

    fn write_digest(&mut self, outputs: &[RoundOutput]) {
        let text = self.world.digest(outputs);
        if !text.is_empty() {
            let _ = self.out.write_all(text.as_bytes());
            let _ = self.out.flush();
        }
    }
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
    let mut out = frames::resume_line(&view);
    out.push_str(&frames::view_line(&view));
    Ok(out)
}

/// One player line, parsed by the table for the table's own dispatch.
enum PlayerCommand<'w, 'l> {
    Close,
    Help,
    Look,
    Map,
    Say(Address<'w>, &'l str),
    Move(&'l str, Option<&'l str>),
}

/// `close` is matched on the raw line, so `player:<id> close` is a move.
fn parse_command<'w, 'l: 'w>(
    world: &'w World,
    seated: &str,
    line: &'l str,
) -> Result<PlayerCommand<'w, 'l>, Refusal> {
    if split_token(line) == ("close", "") {
        return Ok(PlayerCommand::Close);
    }
    let (verb, rest) = split_verb(line, seated)?;
    if verb.is_empty() {
        return Err(Reason::Schema.into());
    }
    if verb == "say" {
        if rest.is_empty() {
            return Err(Reason::Schema.into());
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
        return Ok(PlayerCommand::Say(heard, text));
    }
    if rest.is_empty() {
        return Ok(match verb {
            "help" => PlayerCommand::Help,
            "map" => PlayerCommand::Map,
            "look" => PlayerCommand::Look,
            _ => PlayerCommand::Move(verb, None),
        });
    }
    let (target, extra) = split_token(rest);
    if target.is_empty() || !extra.is_empty() {
        return Err(Reason::Schema.into());
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
