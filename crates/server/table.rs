//! The table, the one owner of the world and its seats: connections, words, say and rounds.
//! Every message it sends but a hello takes the next `seq` from its one counter.

use std::io::Write;
use std::sync::mpsc::{Receiver, Sender};

use serde_json::{Map, Value};
use spookery_engine::{
    emit_tape_record, Address, Presence, Reason, RecipientLine, RoundOutput, TapeRecord, World,
};
use tokio::sync::{oneshot, watch};

use super::frames;
use super::gm::{run_gm_word, Reply};
use super::idle::{self, now_stamp};
use super::seat::{Claim, Conn, Connection, FrameSender, Seats};
use super::Error;
use crate::protocol::{
    self, parse_args, ClaimArgs, ClientBody, ClientMessage, Detail, MoveArgs, NoArgs, SayArgs,
    Seat, ServerBody, ServerMessage, VERSION, WORDS,
};

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

/// Sent from the game port, the MCP endpoint and the idle timer to the table thread.
pub(super) enum Request {
    /// A new connection on `seat`. A GM connection takes the one GM seat.
    Open {
        conn: Conn,
        seat: Seat,
        frames_tx: FrameSender,
        reply: oneshot::Sender<Result<(), Box<ServerMessage>>>,
    },
    /// One client message, or why the game port could not read it.
    Message {
        conn: Conn,
        message: Result<ClientMessage, Rejection>,
        reply: oneshot::Sender<()>,
    },
    /// A connection closed. The table lets go of it, then acks.
    Close {
        conn: Conn,
        ack: oneshot::Sender<()>,
    },
    /// One MCP tool call, as a client message. The reply is every message the table sent
    /// this connection while it ran the word, the word's own reply last.
    ToolCall {
        conn: Conn,
        message: ClientMessage,
        reply: oneshot::Sender<Vec<ServerMessage>>,
    },
    Wake,
    Stop,
}

/// A client message the game port could not read, sent from the game port to the table:
/// the `id` it named, if any, and why.
pub(super) struct Rejection {
    pub(super) id: Option<String>,
    pub(super) code: Reason,
}

/// Tells the table a connection closed, and waits until the table has let go of it.
pub(super) async fn report_close(tx: &Sender<Request>, conn: Conn) {
    let (ack_tx, ack_rx) = oneshot::channel();
    if tx.send(Request::Close { conn, ack: ack_tx }).is_err() {
        return;
    }
    let _ = ack_rx.await;
}

pub(crate) struct Table {
    world: World,
    seats: Seats,
    /// The `seq` of the last message sent.
    seq: u64,
    /// The pack's verbs, which are player words too.
    verbs: Vec<String>,
    /// Each send restarts the idle timer.
    idle: watch::Sender<u64>,
    /// While an MCP tool call runs: its connection, and the messages sent to it so far.
    tool_call: Option<(Conn, Vec<ServerMessage>)>,
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
        let verbs = world.verbs();
        Self {
            world,
            seats: Seats::default(),
            seq: 0,
            verbs,
            idle,
            tool_call: None,
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
                Request::Open {
                    conn,
                    seat,
                    frames_tx,
                    reply,
                } => {
                    let opened = self.open(conn, seat, frames_tx);
                    let took = opened.is_ok();
                    if reply.send(opened).is_err() && took {
                        self.drop_connection(conn);
                    }
                }
                Request::Message {
                    conn,
                    message: received,
                    reply,
                } => {
                    self.handle_message(conn, received);
                    let _ = reply.send(());
                }
                Request::Close { conn, ack } => {
                    self.drop_connection(conn);
                    let _ = ack.send(());
                }
                Request::ToolCall {
                    conn,
                    message,
                    reply,
                } => {
                    self.tool_call = Some((conn, Vec::new()));
                    // An MCP session holds no seat until its event stream opens.
                    if self.seats.connection(conn).is_some() {
                        self.handle_message(conn, Ok(message));
                    } else {
                        let refused = ServerBody::Refused(Reason::NotInSpec.into());
                        self.reply(conn, message.id, refused);
                    }
                    let messages = self.tool_call.take().map(|(_, messages)| messages);
                    let _ = reply.send(messages.unwrap_or_default());
                }
                Request::Wake => handled = self.wake(),
                Request::Stop => {
                    self.seats = Seats::default();
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

    /// Opens the connection and sends its hello. A GM connection while the seat is held, or a
    /// second one for an MCP session that already has its stream, is refused, and the
    /// connection sends that refusal itself.
    fn open(&mut self, conn: Conn, seat: Seat, out: FrameSender) -> Result<(), Box<ServerMessage>> {
        if seat == Seat::Gm && (self.seats.gm.is_some() || self.seats.connection(conn).is_some()) {
            let seq = self.stamp();
            let refused = stamp_message(seq, None, ServerBody::Refused(Reason::TableFull.into()));
            return Err(Box::new(refused));
        }
        if seat == Seat::Gm {
            self.seats.gm = Some(conn);
        }
        self.seats.connections.push(Connection { conn, seat, out });
        let mut words: Vec<String> = WORDS
            .iter()
            .filter(|word| word.seat == seat)
            .map(|word| word.name.to_string())
            .collect();
        let mut open = Vec::new();
        if seat == Seat::Player {
            words.extend(self.verbs.iter().cloned());
            open = self.list_open_pcs();
        }
        let hello = ServerMessage {
            id: None,
            seq: None,
            reply_to: None,
            body: ServerBody::Hello {
                version: VERSION,
                words,
                open,
            },
        };
        self.send(conn, hello);
        Ok(())
    }

    /// The PCs a claim may take now. None once every player seat is held.
    fn list_open_pcs(&self) -> Vec<String> {
        let presence = self.seats.presence();
        if presence.actors_present.len() >= MAX_PLAYER_SEATS {
            return Vec::new();
        }
        self.world.free_pcs(&presence)
    }

    fn handle_message(&mut self, conn: Conn, received: Result<ClientMessage, Rejection>) {
        let Some(seat) = self
            .seats
            .connection(conn)
            .map(|connection| connection.seat)
        else {
            return;
        };
        let ClientMessage { id, body, .. } = match received {
            Ok(message) => message,
            Err(Rejection { id, code }) => {
                return self.reply(conn, id, ServerBody::Refused(code.into()))
            }
        };
        match seat {
            Seat::Player => self.handle_player_word(conn, id, body),
            Seat::Gm => self.handle_gm_word(conn, id, body),
        }
    }

    fn handle_player_word(&mut self, conn: Conn, id: Option<String>, body: ClientBody) {
        let ClientBody { word, args } = body;
        let built_in = WORDS
            .iter()
            .any(|known| known.seat == Seat::Player && known.name == word);
        if !built_in && !self.verbs.contains(&word) {
            return self.reply(conn, id, ServerBody::Refused(Reason::Schema.into()));
        }
        if word == "claim" {
            return self.claim(conn, id, &args);
        }
        let Some(actor_id) = self.seats.claim_of(conn).map(|claim| claim.actor.clone()) else {
            let free = protocol::Refusal {
                code: protocol::Code::Known(Reason::ChoosePc),
                detail: Some(Detail::Many(self.list_open_pcs())),
            };
            return self.reply(conn, id, ServerBody::Refused(free));
        };
        let presence = self.seats.presence();
        let body = match word.as_str() {
            "release" if read::<NoArgs>(&args).is_some() => return self.release_claim(conn, id),
            "say" => match read::<SayArgs>(&args) {
                Some(args) => return self.say_as_player(conn, id, &actor_id, &args),
                None => ServerBody::Refused(Reason::Schema.into()),
            },
            // The view, not text: the client writes the look from it.
            "look" if read::<NoArgs>(&args).is_some() => {
                match self.world.perceive(&presence, &actor_id) {
                    Ok(view) => frames::render_view(&view),
                    Err(code) => ServerBody::Refused(code.into()),
                }
            }
            // Text ready to print, read from the world as it is now.
            "help" if read::<NoArgs>(&args).is_some() => {
                lines_or_refused(self.world.help(&presence, &actor_id))
            }
            "map" if read::<NoArgs>(&args).is_some() => {
                lines_or_refused(self.world.map(&presence, &actor_id))
            }
            "release" | "look" | "help" | "map" => ServerBody::Refused(Reason::Schema.into()),
            _ => match read::<MoveArgs>(&args) {
                Some(args) => return self.play_move(conn, id, &actor_id, &word, args.target),
                None => ServerBody::Refused(Reason::Schema.into()),
            },
        };
        self.reply(conn, id, body);
    }

    /// A claim takes a free PC, or by its token takes back one this player held. The reply
    /// is `claimed`, then a refresh.
    fn claim(&mut self, conn: Conn, id: Option<String>, args: &Map<String, Value>) {
        let args = read::<ClaimArgs>(args).filter(|_| self.seats.claim_of(conn).is_none());
        let fresh = matches!(args, Some(ClaimArgs::Actor { .. }));
        let taken = match args {
            Some(ClaimArgs::Actor { actor }) => self.claim_free_pc(actor),
            Some(ClaimArgs::Token { token, last_seq }) => self.claim_by_token(&token, last_seq),
            None => Err(Reason::Schema),
        };
        let (token, actor, claimed) = match taken {
            Ok(taken) => taken,
            Err(code) => return self.reply(conn, id, ServerBody::Refused(code.into())),
        };
        // After the claim, so the view counts this player as present.
        let [resume, view] = match refresh(&self.world, &self.seats.presence(), &actor) {
            Ok(bodies) => bodies,
            Err(code) => {
                if fresh {
                    self.seats.claims.retain(|claim| claim.token != token);
                }
                return self.reply(conn, id, ServerBody::Refused(code.into()));
            }
        };
        // The reply goes out before this connection drives the claim, so no catch-up
        // carries it.
        self.reply(conn, id, claimed);
        if let Some(claim) = self.seats.claim_of_actor_mut(&actor) {
            claim.conn = Some(conn);
        }
        self.notify(conn, resume);
        self.notify(conn, view);
        if fresh {
            self.announce_presence();
        }
    }

    /// A new claim on a free PC, under a new token, with the says this PC heard before.
    fn claim_free_pc(&mut self, actor: String) -> Result<(String, String, ServerBody), Reason> {
        let presence = self.seats.presence();
        if presence.actors_present.len() >= MAX_PLAYER_SEATS {
            return Err(Reason::TableFull);
        }
        if presence.actors_present.contains(&actor) {
            return Err(Reason::SeatTaken);
        }
        let live = self.world.state().actors.get(&actor);
        if !live.is_some_and(|live| live.is_pc()) {
            return Err(Reason::Schema);
        }
        let scrollback = self.world.player_scrollback(&actor)?;
        let token = uuid::Uuid::new_v4().simple().to_string();
        let claimed = frames::render_claimed(&actor, &token, &scrollback);
        self.seats
            .claims
            .push(Claim::new(token.clone(), actor.clone()));
        Ok((token, actor, claimed))
    }

    /// The held claim `token` names, with what was sent to it after `last_seq`. A connection
    /// that still drives it lets go.
    fn claim_by_token(
        &mut self,
        token: &str,
        last_seq: u64,
    ) -> Result<(String, String, ServerBody), Reason> {
        let claim = self
            .seats
            .claims
            .iter_mut()
            .find(|claim| claim.token == token)
            .ok_or(Reason::UnknownId)?;
        claim.conn = None;
        let (missed, gap) = claim.messages_after(last_seq);
        let claimed = ServerBody::Claimed {
            actor: claim.actor.clone(),
            token: token.to_string(),
            scrollback: Vec::new(),
            missed,
            gap,
        };
        Ok((token.to_string(), claim.actor.clone(), claimed))
    }

    fn release_claim(&mut self, conn: Conn, id: Option<String>) {
        self.seats.claims.retain(|claim| claim.conn != Some(conn));
        self.reply(conn, id, ServerBody::Released);
        self.announce_presence();
    }

    /// A player's `place` say is heard where that player stands, and nowhere else.
    fn say_as_player(&mut self, conn: Conn, id: Option<String>, actor_id: &str, args: &SayArgs) {
        let here = self
            .world
            .state()
            .actors
            .get(actor_id)
            .map(|actor| actor.place.clone());
        let heard = match args {
            SayArgs::Table { .. } => Address::Table,
            SayArgs::Place { place, .. } if here.as_ref() == Some(place) => Address::Place(place),
            SayArgs::Actor { actor, .. } => Address::Actor(actor),
            SayArgs::Place { .. } => {
                return self.reply(conn, id, ServerBody::Refused(Reason::Schema.into()));
            }
        };
        let presence = self.seats.presence();
        match self
            .world
            .say_as_player(&presence, actor_id, heard, args.text())
        {
            Ok(said) => self.deliver_say(&said, Some((conn, id))),
            Err(code) => self.reply(conn, id, ServerBody::Refused(code.into())),
        }
    }

    /// With a GM seated, a move waits for `commit`, and the GM hears it queued. Without
    /// one it commits its own round, and a busy PC's move plays the rounds its action still
    /// needs. Either way the reply follows what the move wrote.
    fn play_move(
        &mut self,
        conn: Conn,
        id: Option<String>,
        actor_id: &str,
        verb: &str,
        target: Option<String>,
    ) {
        let gm = self.seats.gm;
        let presence = self.seats.presence();
        let target = target.filter(|target| !target.is_empty());
        let reply = match self
            .world
            .queue_player_move(&presence, actor_id, verb, target.as_deref())
        {
            Ok(queued) => ServerBody::Queued {
                intent: queued.id,
                actor: queued.actor_id,
                move_name: queued.move_name,
                target,
            },
            Err(refused) if gm.is_none() && refused.code == Reason::Busy => {
                ServerBody::Refused(frames::render_refused_move(&refused))
            }
            Err(refused) => {
                return self.reply(
                    conn,
                    id,
                    ServerBody::Refused(frames::render_refused_move(&refused)),
                );
            }
        };
        if let Some(gm) = gm {
            let seq = self.stamp();
            self.send_to_pc(actor_id, stamp_message(seq, id, reply.clone()));
            return self.send(gm, stamp_message(seq, None, reply));
        }
        match commit_then_fast_forward(&mut self.world, &presence) {
            Ok(outputs) => {
                self.reply(conn, id, reply);
                self.after_rounds(&outputs);
            }
            Err(code) => self.reply(conn, id, ServerBody::Refused(code.into())),
        }
    }

    fn handle_gm_word(&mut self, conn: Conn, id: Option<String>, body: ClientBody) {
        if self.seats.gm != Some(conn) {
            return self.reply(conn, id, ServerBody::Refused(Reason::NotInSpec.into()));
        }
        let presence = self.seats.presence();
        let reply = run_gm_word(&mut self.world, &presence, &body.word, &body.args)
            .unwrap_or_else(Reply::refused);
        self.log_refused(&body.word, &reply);
        self.after_rounds(&reply.outputs);
        // A say's reply is the GM's copy of it.
        match &reply.said {
            Some(said) => self.deliver_say(said, Some((conn, id))),
            None => self.reply(conn, id, reply.body),
        }
        if reply.release_seat {
            self.release_gm();
        }
    }

    fn log_refused(&mut self, word: &str, reply: &Reply) {
        if let ServerBody::Refused(refused) = &reply.body {
            let _ = writeln!(self.out, "refused {word} {}", refused.code.as_str());
            let _ = self.out.flush();
        }
    }

    /// Frees the GM seat and says so on the host log, so the operator sees the seat is open.
    fn release_gm(&mut self) {
        self.seats.gm = None;
        let _ = writeln!(self.out, "gm seat free");
        let _ = self.out.flush();
    }

    fn wake(&mut self) -> Result<(), Reason> {
        // A seated table plays its own rounds. A claim whose connection dropped still seats it.
        if !self.seats.claims.is_empty() || self.seats.gm.is_some() {
            return Ok(());
        }
        if idle::catch_up_if_due(&mut self.world)? {
            self.restart_idle_timer();
        }
        Ok(())
    }

    /// The one order after rounds: a `round` to every seat, then each player's checks,
    /// refresh and lines, then the digest, then the idle timer restarts.
    fn after_rounds(&mut self, outputs: &[RoundOutput]) {
        if outputs.is_empty() {
            return;
        }
        let players: Vec<String> = self
            .seats
            .claims
            .iter()
            .map(|claim| claim.actor.clone())
            .collect();
        for commit in outputs.iter().filter_map(|output| output.commit.as_ref()) {
            let seq = self.stamp();
            let body = ServerBody::Round {
                round_index: commit.round_index,
                world_time: commit.world_time,
            };
            let round = stamp_message(seq, None, body);
            for actor_id in &players {
                self.send_to_pc(actor_id, round.clone());
            }
            if let Some(gm) = self.seats.gm {
                self.send(gm, round);
            }
        }
        let presence = self.seats.presence();
        for actor_id in players {
            let mut bodies: Vec<ServerBody> = outputs
                .iter()
                .flat_map(|output| frames::render_checks(&output.reports, &actor_id))
                .collect();
            match refresh(&self.world, &presence, &actor_id) {
                Ok(refreshed) => bodies.extend(refreshed),
                Err(code) => bodies.push(ServerBody::Refused(code.into())),
            }
            for output in outputs {
                for text in narrate_lines_for(&output.lines, &actor_id) {
                    bodies.push(ServerBody::Lines { text });
                }
                for item in &output.seen {
                    if item.viewer == actor_id && !item.line.is_empty() {
                        bodies.push(ServerBody::Lines {
                            text: item.line.clone(),
                        });
                    }
                }
            }
            for body in bodies {
                self.notify_pc(&actor_id, body);
            }
        }
        self.write_digest(outputs);
        self.restart_idle_timer();
    }

    fn restart_idle_timer(&self) {
        self.idle
            .send_modify(|count| *count = count.wrapping_add(1));
    }

    /// The one say path: the host's tape line, then one `say` with one `seq` to the GM and
    /// to each player who hears it. The sender's copy is its reply.
    fn deliver_say(&mut self, said: &TapeRecord, sender: Option<(Conn, Option<String>)>) {
        let _ = writeln!(self.out, "{}", emit_tape_record(said));
        let _ = self.out.flush();
        let seq = self.stamp();
        let (sender, id) = sender.unzip();
        let id = id.flatten();
        let reply_to = |conn: Conn| (sender == Some(conn)).then(|| id.clone()).flatten();
        let hears: Vec<(String, Option<Conn>)> = self
            .seats
            .claims
            .iter()
            .filter(|claim| {
                said.recipients.contains(&claim.actor)
                    || claim.conn.is_some_and(|conn| sender == Some(conn))
            })
            .map(|claim| (claim.actor.clone(), claim.conn))
            .collect();
        if let Some(gm) = self.seats.gm {
            let body = frames::render_say(said, true);
            self.send(gm, stamp_message(seq, reply_to(gm), body));
        }
        for (actor_id, conn) in hears {
            let body = frames::render_say(said, false);
            let message = stamp_message(seq, conn.and_then(reply_to), body);
            self.send_to_pc(&actor_id, message);
        }
    }

    /// Tells the GM who is present now, and the host.
    fn announce_presence(&mut self) {
        if let Some(gm) = self.seats.gm {
            let seq = self.stamp();
            let body = ServerBody::Presence {
                actors_present: self.seats.presence().actors_present,
            };
            self.send(gm, stamp_message(seq, None, body));
        }
        self.write_digest(&[]);
    }

    /// The next `seq`.
    fn stamp(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    /// The reply to the message `id` named. One to a connection that drives a claim goes
    /// to that PC.
    fn reply(&mut self, conn: Conn, id: Option<String>, body: ServerBody) {
        let seq = self.stamp();
        let message = stamp_message(seq, id, body);
        match self.seats.claim_of(conn).map(|claim| claim.actor.clone()) {
            Some(actor_id) => self.send_to_pc(&actor_id, message),
            None => self.send(conn, message),
        }
    }

    /// A message that answers nothing.
    fn notify(&mut self, conn: Conn, body: ServerBody) {
        self.reply(conn, None, body);
    }

    /// A message that answers nothing, to a PC.
    fn notify_pc(&mut self, actor_id: &str, body: ServerBody) {
        let seq = self.stamp();
        self.send_to_pc(actor_id, stamp_message(seq, None, body));
    }

    /// Keeps the message on the PC's claim for a reconnect, and sends it if a connection
    /// drives that claim.
    fn send_to_pc(&mut self, actor_id: &str, message: ServerMessage) {
        let Some(claim) = self.seats.claim_of_actor_mut(actor_id) else {
            return;
        };
        claim.keep(&message);
        if let Some(conn) = claim.conn {
            self.send(conn, message);
        }
    }

    /// A message to an MCP tool call's connection goes in that call's result. A full or
    /// closed queue drops that connection. The table does not wait.
    fn send(&mut self, conn: Conn, message: ServerMessage) {
        if let Some((_, messages)) = self.tool_call.as_mut().filter(|(call, _)| *call == conn) {
            return messages.push(message);
        }
        let bytes = message.to_text().into_bytes();
        if self
            .seats
            .connection(conn)
            .is_some_and(|connection| !connection.out.send(bytes))
        {
            self.drop_connection(conn);
        }
    }

    /// Drops a connection. Dropping it cancels its socket. A claim it drove is held
    /// for its token, and a GM seat it held is free.
    fn drop_connection(&mut self, conn: Conn) {
        if self.seats.gm == Some(conn) {
            self.release_gm();
        }
        if let Some(claim) = self.seats.claim_of_mut(conn) {
            claim.conn = None;
        }
        self.seats.take(conn);
    }

    fn write_digest(&mut self, outputs: &[RoundOutput]) {
        let text = self.world.digest(outputs);
        if !text.is_empty() {
            let _ = self.out.write_all(text.as_bytes());
            let _ = self.out.flush();
        }
    }
}

fn stamp_message(seq: u64, reply_to: Option<String>, body: ServerBody) -> ServerMessage {
    ServerMessage {
        id: None,
        seq: Some(seq),
        reply_to,
        body,
    }
}

/// A help or map reply, or the refusal when the world can't answer.
fn lines_or_refused(text: Result<String, Reason>) -> ServerBody {
    match text {
        Ok(text) => ServerBody::Lines { text },
        Err(code) => ServerBody::Refused(code.into()),
    }
}

/// A word's arguments, or `None` when they do not fit its type.
fn read<T: serde::de::DeserializeOwned>(args: &Map<String, Value>) -> Option<T> {
    parse_args(Some(args)).ok()
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

fn refresh(world: &World, presence: &Presence, actor_id: &str) -> Result<[ServerBody; 2], Reason> {
    let view = world.perceive(presence, actor_id)?;
    Ok([frames::render_resume(&view), frames::render_view(&view)])
}
