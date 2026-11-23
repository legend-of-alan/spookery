//! The table, the one owner of the session and its seats: connections, commands, say and rounds.
//! Every message it sends but a hello takes the next `seq` from its one counter.

use std::collections::BTreeMap;
use std::io::Write;
use std::sync::mpsc::{Receiver, Sender};

use serde_json::{Map, Value};
use spookery_engine::{emit_tape_record, ActorId, Presence, Reason, RecipientLine, TapeRecord};
use tokio::sync::{oneshot, watch};

use super::frames;
use super::gm::{self, Reply};
use super::idle::{self, now_stamp};
use super::seat::{Claim, ConnId, Connection, FrameSender, Seats};
use super::Error;
use crate::protocol::{
    is_built_in, parse_args, Audience, ClaimArgs, ClientBody, ClientMessage, MoveArgs, NoArgs,
    SayArgs, Seat, ServerBody, ServerMessage, COMMANDS, VERSION,
};

/// The host's cap. A pack may define more PCs than this.
const MAX_PLAYER_SEATS: usize = 3;

/// Commit a queued round, then fast-forward while every seated PC is busy.
/// An empty queue only fast-forwards.
pub(crate) fn commit_then_fast_forward(
    session: &mut spookery_engine::Session,
    presence: &Presence,
) -> Result<Vec<spookery_engine::round::Output>, Reason> {
    let wall = now_stamp();
    session.commit_then_fast_forward(presence, &wall)
}

/// Sent from the game port, the MCP endpoint and the idle timer to the table thread.
pub(super) enum Request {
    /// A new connection on `seat`. A GM connection takes the one GM seat.
    Open {
        conn: ConnId,
        seat: Seat,
        frames_tx: FrameSender,
        reply: oneshot::Sender<Result<(), Box<ServerMessage>>>,
    },
    /// One client message, or why the game port could not read it.
    Message {
        conn: ConnId,
        message: Result<ClientMessage, Rejection>,
        reply: oneshot::Sender<()>,
    },
    /// A connection closed. The table lets go of it, then acks.
    Close {
        conn: ConnId,
        ack: oneshot::Sender<()>,
    },
    /// One MCP tool call, as a client message. The reply is every message the table sent
    /// this connection while it ran the command, the command's own reply last.
    ToolCall {
        conn: ConnId,
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
pub(super) async fn report_close(tx: &Sender<Request>, conn: ConnId) {
    let (ack_tx, ack_rx) = oneshot::channel();
    if tx.send(Request::Close { conn, ack: ack_tx }).is_err() {
        return;
    }
    let _ = ack_rx.await;
}

pub(crate) struct Table {
    session: spookery_engine::Session,
    seats: Seats,
    /// The `seq` of the last message sent.
    seq: u64,
    /// The pack's verbs, which are player commands too.
    verbs: Vec<String>,
    /// Each send restarts the idle timer.
    idle: watch::Sender<u64>,
    /// While an MCP tool call runs: its connection, and the messages sent to it so far.
    tool_call: Option<(ConnId, Vec<ServerMessage>)>,
    out: Box<dyn Write>,
}

impl Table {
    pub(crate) fn new(
        session: spookery_engine::Session,
        idle: watch::Sender<u64>,
        out: Box<dyn Write>,
    ) -> Self {
        let verbs = session.verbs();
        Self {
            session,
            seats: Seats::default(),
            seq: 0,
            verbs,
            idle,
            tool_call: None,
            out,
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
            handled.map_err(Error::Reason)?;
        }
    }

    /// Opens the connection and sends its hello. A GM connection while the seat is held, or a
    /// second one for an MCP session that already has its stream, is refused, and the
    /// connection sends that refusal itself.
    fn open(
        &mut self,
        conn: ConnId,
        seat: Seat,
        out: FrameSender,
    ) -> Result<(), Box<ServerMessage>> {
        if seat == Seat::Gm && (self.seats.gm.is_some() || self.seats.connection(conn).is_some()) {
            tracing::debug!(conn_id = %conn, code = Reason::TableFull.as_str(), "GM seat refused");
            let seq = self.next_seq();
            let refused = stamp_message(seq, None, ServerBody::Refused(Reason::TableFull.into()));
            return Err(Box::new(refused));
        }
        if seat == Seat::Gm {
            tracing::info!(conn_id = %conn, "GM seated");
            self.seats.gm = Some(conn);
        }
        tracing::debug!(conn_id = %conn, ?seat, "connection opened");
        self.seats.connections.push(Connection {
            id: conn,
            seat,
            out,
        });
        let mut commands: Vec<String> = COMMANDS
            .iter()
            .filter(|command| command.seat == seat)
            .map(|command| command.id.to_string())
            .collect();
        let (mut open, mut taken, mut pc_names) = (Vec::new(), Vec::new(), BTreeMap::new());
        if seat == Seat::Player {
            commands.extend(self.verbs.iter().cloned());
            (open, taken) = self.open_and_taken();
            let canon = self.session.canon();
            let ids = open.iter().chain(&taken);
            let name = |id: &String| canon.cast[id.as_str()].name.clone();
            pc_names = ids.map(|id| (id.clone(), name(id))).collect();
        }
        let hello = ServerMessage {
            id: None,
            seq: None,
            reply_to: None,
            body: ServerBody::Hello {
                version: VERSION,
                commands,
                open,
                taken,
                pc_names,
            },
        };
        self.send(conn, hello);
        Ok(())
    }

    /// The hello's `open` and `taken`: the PCs nobody holds now, and the PCs held now.
    fn open_and_taken(&self) -> (Vec<String>, Vec<String>) {
        let presence = self.seats.presence();
        self.session.pcs().map(ActorId::to_string).partition(|id| {
            !presence
                .actors_present
                .iter()
                .any(|actor| actor == id.as_str())
        })
    }

    fn handle_message(&mut self, conn: ConnId, received: Result<ClientMessage, Rejection>) {
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
        tracing::debug!(conn_id = %conn, ?seat, command_id = %body.kind, "command received");
        match seat {
            Seat::Player => self.handle_player_command(conn, id, body),
            Seat::Gm => self.handle_gm_command(conn, id, body),
        }
    }

    fn handle_player_command(&mut self, conn: ConnId, id: Option<String>, body: ClientBody) {
        let ClientBody {
            kind: command,
            args,
        } = body;
        let built_in = is_built_in(Seat::Player, &command);
        if !built_in && !self.verbs.contains(&command) {
            return self.reply(conn, id, ServerBody::Refused(Reason::Schema.into()));
        }
        if command == "claim" {
            return self.claim(conn, id, &args);
        }
        let Some(actor_id) = self.seats.claim_of(conn).map(|claim| claim.actor.clone()) else {
            return self.reply(conn, id, ServerBody::Refused(Reason::ChoosePc.into()));
        };
        let presence = self.seats.presence();
        let body = match command.as_str() {
            "release" if read::<NoArgs>(&args).is_some() => return self.release_claim(conn, id),
            "say" => match read::<SayArgs>(&args).filter(|args| args.npc_id.is_none()) {
                Some(SayArgs {
                    to: Some(to), text, ..
                }) => return self.say_as_player(conn, id, &actor_id, &to, &text),
                _ => ServerBody::Refused(Reason::Schema.into()),
            },
            "look" if read::<NoArgs>(&args).is_some() => {
                match self.session.perceive(&presence, actor_id.as_str()) {
                    Ok(view) => frames::render_view(&view),
                    Err(code) => ServerBody::Refused(code.into()),
                }
            }
            "help" if read::<NoArgs>(&args).is_some() => {
                match self.session.help(&presence, &actor_id) {
                    Ok(commands) => ServerBody::Help { commands },
                    Err(code) => ServerBody::Refused(code.into()),
                }
            }
            "map" if read::<NoArgs>(&args).is_some() => match self.session.map(&actor_id) {
                Ok(places) => ServerBody::Map { places },
                Err(code) => ServerBody::Refused(code.into()),
            },
            "release" | "look" | "help" | "map" => ServerBody::Refused(Reason::Schema.into()),
            _ => match read::<MoveArgs>(&args) {
                Some(args) => return self.play_move(conn, id, &actor_id, &command, args.target),
                None => ServerBody::Refused(Reason::Schema.into()),
            },
        };
        self.reply(conn, id, body);
    }

    /// A claim takes a free PC, or by its token takes back one this player held. The reply
    /// is `claimed`, then a refresh.
    fn claim(&mut self, conn: ConnId, id: Option<String>, args: &Map<String, Value>) {
        let args = read::<ClaimArgs>(args).filter(|_| self.seats.claim_of(conn).is_none());
        let fresh = matches!(args, Some(ClaimArgs::Actor { .. }));
        let taken = match args {
            Some(ClaimArgs::Actor { actor }) => self.claim_free_pc(&actor),
            Some(ClaimArgs::Token { token, last_seq }) => self.claim_by_token(&token, last_seq),
            None => Err(Reason::Schema),
        };
        let (token, actor, claim_reply) = match taken {
            Ok(taken) => taken,
            Err(code) => return self.reply(conn, id, ServerBody::Refused(code.into())),
        };
        let [resume, view] = match refresh(&self.session, &self.seats.presence(), &actor) {
            Ok(bodies) => bodies,
            Err(code) => {
                if fresh {
                    self.seats.claims.retain(|claim| claim.token != token);
                }
                return self.reply(conn, id, ServerBody::Refused(code.into()));
            }
        };
        self.reply(conn, id, claim_reply);
        tracing::info!(conn_id = %conn, actor_id = %actor, by_token = !fresh, "PC claimed");
        if let Some(claim) = self.seats.claim_of_actor_mut(&actor) {
            claim.conn = Some(conn);
        }
        self.notify(conn, resume);
        self.notify(conn, view);
        if fresh {
            self.announce_presence();
        }
    }

    /// A new claim on a free PC, named by id or name, under a new token, with the speech this PC
    /// heard before.
    fn claim_free_pc(&mut self, text: &str) -> Result<(String, ActorId, ServerBody), Reason> {
        let presence = self.seats.presence();
        if presence.actors_present.len() >= MAX_PLAYER_SEATS {
            return Err(Reason::TableFull);
        }
        let actor = self.session.find_pc(text)?.ok_or(Reason::Schema)?;
        if presence.actors_present.contains(&actor) {
            return Err(Reason::SeatTaken);
        }
        let scrollback = self.session.player_scrollback(&actor)?;
        let claim = Claim::new(uuid::Uuid::new_v4().simple().to_string(), actor.clone());
        let claim_reply = frames::render_claim_reply(self.session.world(), &claim, &scrollback, 0);
        let token = claim.token.clone();
        self.seats.claims.push(claim);
        Ok((token, actor, claim_reply))
    }

    /// The held claim `token` names, with what was sent to it after `last_seq`. A connection
    /// that still drives it lets go.
    fn claim_by_token(
        &mut self,
        token: &str,
        last_seq: u64,
    ) -> Result<(String, ActorId, ServerBody), Reason> {
        let claim = self
            .seats
            .claims
            .iter_mut()
            .find(|claim| claim.token == token)
            .ok_or(Reason::UnknownId)?;
        claim.conn = None;
        let claim_reply = frames::render_claim_reply(self.session.world(), claim, &[], last_seq);
        Ok((token.to_string(), claim.actor.clone(), claim_reply))
    }

    fn release_claim(&mut self, conn: ConnId, id: Option<String>) {
        if let Some(claim) = self.seats.claim_of(conn) {
            tracing::info!(conn_id = %conn, actor_id = %claim.actor, "PC released");
        }
        self.seats.claims.retain(|claim| claim.conn != Some(conn));
        self.reply(conn, id, ServerBody::Released);
        self.announce_presence();
    }

    fn say_as_player(
        &mut self,
        conn: ConnId,
        id: Option<String>,
        actor_id: &ActorId,
        to: &Audience,
        text: &str,
    ) {
        let presence = self.seats.presence();
        match self.session.say_as_player(&presence, actor_id, to, text) {
            Ok(speech) => self.deliver_speech(&speech, Some((conn, id))),
            Err(refusal) => self.reply(conn, id, ServerBody::Refused(refusal)),
        }
    }

    /// With a GM seated, a move waits for `commit`, and the GM hears it queued. Without
    /// one it commits its own round, and a busy PC's move plays the rounds its action still
    /// needs. Either way the reply follows what the move wrote.
    fn play_move(
        &mut self,
        conn: ConnId,
        id: Option<String>,
        actor_id: &ActorId,
        verb: &str,
        target: Option<String>,
    ) {
        let gm = self.seats.gm;
        let presence = self.seats.presence();
        let target = target.filter(|target| !target.is_empty());
        let reply =
            match self
                .session
                .queue_player_move(&presence, actor_id, verb, target.as_deref())
            {
                Ok((queued, target)) => ServerBody::Queued {
                    intent: queued.id,
                    actor: queued.actor_id,
                    move_id: queued.move_id,
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
            let seq = self.next_seq();
            self.send_to_pc(actor_id, stamp_message(seq, id, reply.clone()));
            return self.send(gm, stamp_message(seq, None, reply));
        }
        match commit_then_fast_forward(&mut self.session, &presence) {
            Ok(outputs) => {
                self.reply(conn, id, reply);
                self.after_rounds(&outputs);
            }
            Err(code) => self.reply(conn, id, ServerBody::Refused(code.into())),
        }
    }

    fn handle_gm_command(&mut self, conn: ConnId, id: Option<String>, body: ClientBody) {
        if self.seats.gm != Some(conn) {
            return self.reply(conn, id, ServerBody::Refused(Reason::NotInSpec.into()));
        }
        let presence = self.seats.presence();
        let reply = gm::run(&mut self.session, &presence, &body.kind, &body.args)
            .unwrap_or_else(Reply::refused);
        self.after_rounds(&reply.outputs);
        match &reply.speech {
            Some(speech) => self.deliver_speech(speech, Some((conn, id))),
            None => self.reply(conn, id, reply.body),
        }
        if reply.release_seat {
            self.release_gm();
        }
    }

    /// Frees the GM seat and says so on the host console, so the operator sees the seat is open.
    fn release_gm(&mut self) {
        self.seats.gm = None;
        let _ = writeln!(self.out, "gm seat free");
        let _ = self.out.flush();
    }

    /// Plays the idle rounds due at an empty table. A claim whose connection dropped still
    /// seats its player.
    fn wake(&mut self) -> Result<(), Reason> {
        if !self.seats.claims.is_empty() || self.seats.gm.is_some() {
            return Ok(());
        }
        if idle::catch_up_if_due(&mut self.session)? {
            self.restart_idle_timer();
        }
        Ok(())
    }

    /// The one order after rounds: a `round` to every seat, then each player's checks,
    /// refresh and lines, then the digest, then the idle timer restarts.
    fn after_rounds(&mut self, outputs: &[spookery_engine::round::Output]) {
        if outputs.is_empty() {
            return;
        }
        let players: Vec<ActorId> = self
            .seats
            .claims
            .iter()
            .map(|claim| claim.actor.clone())
            .collect();
        for commit in outputs.iter().filter_map(|output| output.commit.as_ref()) {
            let seq = self.next_seq();
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
            match refresh(&self.session, &presence, &actor_id) {
                Ok(refreshed) => bodies.extend(refreshed),
                Err(code) => bodies.push(ServerBody::Refused(code.into())),
            }
            for output in outputs {
                for text in narrate_lines_for(&output.lines, &actor_id) {
                    bodies.push(ServerBody::Lines { text });
                }
                for item in &output.seen {
                    if item.viewer == actor_id && !item.changes.is_empty() {
                        bodies.push(ServerBody::Changes {
                            viewer: actor_id.to_string(),
                            changes: item.changes.clone(),
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

    /// The one speech path: the host's tape line, then one `say` with one `seq` to the GM and
    /// to each player who hears it. The sender's copy is its reply.
    fn deliver_speech(&mut self, speech: &TapeRecord, sender: Option<(ConnId, Option<String>)>) {
        let _ = writeln!(self.out, "{}", emit_tape_record(speech));
        let _ = self.out.flush();
        let seq = self.next_seq();
        let (sender, id) = sender.unzip();
        let id = id.flatten();
        let reply_to = |conn: ConnId| (sender == Some(conn)).then(|| id.clone()).flatten();
        let hears: Vec<(ActorId, Option<ConnId>)> = self
            .seats
            .claims
            .iter()
            .filter(|claim| {
                speech.recipient_ids.iter().any(|id| claim.actor == **id)
                    || claim.conn.is_some_and(|conn| sender == Some(conn))
            })
            .map(|claim| (claim.actor.clone(), claim.conn))
            .collect();
        if let Some(gm) = self.seats.gm {
            let body = frames::render_speech(speech, true);
            self.send(gm, stamp_message(seq, reply_to(gm), body));
        }
        for (actor_id, conn) in hears {
            let body = frames::render_speech(speech, false);
            let message = stamp_message(seq, conn.and_then(reply_to), body);
            self.send_to_pc(&actor_id, message);
        }
    }

    /// Tells the GM who is present now, and the host.
    fn announce_presence(&mut self) {
        if let Some(gm) = self.seats.gm {
            let seq = self.next_seq();
            let body = ServerBody::Presence {
                actors_present: self
                    .seats
                    .presence()
                    .actors_present
                    .into_iter()
                    .map(String::from)
                    .collect(),
            };
            self.send(gm, stamp_message(seq, None, body));
        }
        self.write_digest(&[]);
    }

    /// The next `seq`.
    fn next_seq(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    /// The reply to the message `id` named. One to a connection that drives a claim goes
    /// to that PC.
    fn reply(&mut self, conn: ConnId, id: Option<String>, body: ServerBody) {
        if let ServerBody::Refused(refused) = &body {
            tracing::debug!(conn_id = %conn, code = refused.code.as_str(), "refused");
        }
        let seq = self.next_seq();
        let message = stamp_message(seq, id, body);
        match self.seats.claim_of(conn).map(|claim| claim.actor.clone()) {
            Some(actor_id) => self.send_to_pc(&actor_id, message),
            None => self.send(conn, message),
        }
    }

    /// A message that answers nothing.
    fn notify(&mut self, conn: ConnId, body: ServerBody) {
        self.reply(conn, None, body);
    }

    /// A message that answers nothing, to a PC.
    fn notify_pc(&mut self, actor_id: &ActorId, body: ServerBody) {
        let seq = self.next_seq();
        self.send_to_pc(actor_id, stamp_message(seq, None, body));
    }

    /// Keeps the message on the PC's claim for a reconnect, and sends it if a connection
    /// drives that claim.
    fn send_to_pc(&mut self, actor_id: &ActorId, message: ServerMessage) {
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
    fn send(&mut self, conn: ConnId, message: ServerMessage) {
        if let Some((_, messages)) = self.tool_call.as_mut().filter(|(call, _)| *call == conn) {
            return messages.push(message);
        }
        let bytes = message.to_text().into_bytes();
        if self
            .seats
            .connection(conn)
            .is_some_and(|connection| !connection.out.send(bytes))
        {
            tracing::debug!(conn_id = %conn, "outbound queue full or closed");
            self.drop_connection(conn);
        }
    }

    /// Drops a connection. Dropping it cancels its socket. A claim it drove is held
    /// for its token, and a GM seat it held is free.
    fn drop_connection(&mut self, conn: ConnId) {
        tracing::debug!(conn_id = %conn, "connection closed");
        if self.seats.gm == Some(conn) {
            self.release_gm();
        }
        if let Some(claim) = self.seats.claim_of_mut(conn) {
            claim.conn = None;
        }
        self.seats.take(conn);
    }

    fn write_digest(&mut self, outputs: &[spookery_engine::round::Output]) {
        let text = self.session.digest(outputs);
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

/// A command's arguments, or `None` when they do not fit its type.
fn read<T: serde::de::DeserializeOwned>(args: &Map<String, Value>) -> Option<T> {
    parse_args(Some(args)).ok()
}

fn narrate_lines_for(lines: &[RecipientLine], viewer: &ActorId) -> Vec<String> {
    let mut out = Vec::new();
    for line in lines {
        if line.to == *viewer && !out.contains(&line.text) {
            out.push(line.text.clone());
        }
    }
    out
}

fn refresh(
    session: &spookery_engine::Session,
    presence: &Presence,
    actor_id: &ActorId,
) -> Result<[ServerBody; 2], Reason> {
    let view = session.perceive(presence, actor_id.as_str())?;
    Ok([frames::render_resume(&view), frames::render_view(&view)])
}
