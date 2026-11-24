//! The router reads one request at a time and returns the messages it sends, each addressed to a
//! connection. Every message it sends but a hello takes the next `seq` from its one counter.

use std::collections::BTreeMap;
use std::io::Write;

use serde_json::{Map, Value};
use spookery_protocol::{
    is_built_in, parse_args, Audience, ClaimArgs, ClientBody, ClientMessage, MoveArgs, NoArgs,
    SayArgs, Seat, ServerBody, ServerMessage, COMMANDS, VERSION,
};

use super::gm::{self, Reply};
use super::message_body;
use super::participants::{Claim, ConnId, Connection, Participants};
use crate::{emit_tape_record, ActorId, Presence, Reason, RecipientLine, Session, TapeRecord};

/// The router's cap. A pack may define more PCs than this.
const MAX_PLAYER_SEATS: usize = 3;

/// Sent from a network adapter, such as engine-net's game port, to the router: a client message
/// the adapter could not read, the `id` it named, if any, and why.
pub struct Rejection {
    pub id: Option<String>,
    pub code: Reason,
}

/// Sent from the router back to whatever drives it, such as engine-net's host thread or an
/// in-process host: what one request made the router send.
#[derive(Default)]
pub struct Outbox {
    /// Each message and the connection it goes to, in send order. A caller that cannot queue
    /// one closes that connection with `Router::close`.
    pub messages: Vec<(ConnId, ServerMessage)>,
    /// Whether rounds were played, so an idle timer starts its wait again.
    pub restart_idle: bool,
}

/// The one owner of the session and who is seated: connections, commands, say and rounds.
pub struct Router {
    session: Session,
    participants: Participants,
    /// The `seq` of the last message sent.
    seq: u64,
    /// The pack's verbs, which are player commands too.
    verbs: Vec<String>,
    /// What the request being handled has sent so far.
    outbox: Outbox,
    out: Box<dyn Write>,
}

impl Router {
    /// A router for `session`. Its console lines, speech and digests go to `out`.
    pub fn new(session: Session, out: Box<dyn Write>) -> Self {
        let verbs = session.verbs();
        Self {
            session,
            participants: Participants::default(),
            seq: 0,
            verbs,
            outbox: Outbox::default(),
            out,
        }
    }

    /// Opens the connection and sends its hello. A GM connection while the seat is held, or a
    /// second one for an MCP session that already has its stream, is refused, and the caller
    /// sends that refusal itself.
    pub fn open(&mut self, conn: ConnId, seat: Seat) -> Result<Outbox, Box<ServerMessage>> {
        if seat == Seat::Gm
            && (self.participants.gm.is_some() || self.participants.connection(conn).is_some())
        {
            tracing::debug!(conn_id = %conn, code = Reason::TableFull.as_str(), "GM seat refused");
            let seq = self.next_seq();
            let refused = stamp_message(seq, None, ServerBody::Refused(Reason::TableFull.into()));
            return Err(Box::new(refused));
        }
        if seat == Seat::Gm {
            tracing::info!(conn_id = %conn, "GM seated");
            self.participants.gm = Some(conn);
        }
        tracing::debug!(conn_id = %conn, ?seat, "connection opened");
        self.participants
            .connections
            .push(Connection { id: conn, seat });
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
        Ok(self.take_outbox())
    }

    /// One client message from `conn`, or why the caller could not read it. `wall` stamps any
    /// round it plays.
    pub fn receive(
        &mut self,
        conn: ConnId,
        received: Result<ClientMessage, Rejection>,
        wall: &str,
    ) -> Outbox {
        self.handle_message(conn, received, wall);
        self.take_outbox()
    }

    /// One MCP tool call, as a client message. A connection that is not open is refused.
    pub fn call_tool(&mut self, conn: ConnId, message: ClientMessage, wall: &str) -> Outbox {
        if self.participants.connection(conn).is_some() {
            self.handle_message(conn, Ok(message), wall);
        } else {
            let refused = ServerBody::Refused(Reason::NotInSpec.into());
            self.reply(conn, message.id, refused);
        }
        self.take_outbox()
    }

    /// Lets go of a connection that closed. A claim it drove is held for its token, and a GM
    /// seat it held is free.
    pub fn close(&mut self, conn: ConnId) {
        tracing::debug!(conn_id = %conn, "connection closed");
        if self.participants.gm == Some(conn) {
            self.release_gm();
        }
        if let Some(claim) = self.participants.claim_of_mut(conn) {
            claim.conn = None;
        }
        self.participants.remove(conn);
    }

    /// Plays the idle rounds due while every seat is empty. A claim whose connection dropped
    /// still seats its player. `catch_up` plays them and says whether world time moved.
    pub fn wake(
        &mut self,
        catch_up: impl FnOnce(&mut Session) -> Result<bool, Reason>,
    ) -> Result<Outbox, Reason> {
        let seated = !self.participants.claims.is_empty() || self.participants.gm.is_some();
        if !seated && catch_up(&mut self.session)? {
            self.outbox.restart_idle = true;
        }
        Ok(self.take_outbox())
    }

    fn take_outbox(&mut self) -> Outbox {
        std::mem::take(&mut self.outbox)
    }

    /// The hello's `open` and `taken`: the PCs nobody holds now, and the PCs held now.
    fn open_and_taken(&self) -> (Vec<String>, Vec<String>) {
        let presence = self.participants.presence();
        self.session.pcs().map(ActorId::to_string).partition(|id| {
            !presence
                .actors_present
                .iter()
                .any(|actor_id| actor_id == id.as_str())
        })
    }

    fn handle_message(
        &mut self,
        conn: ConnId,
        received: Result<ClientMessage, Rejection>,
        wall: &str,
    ) {
        let Some(seat) = self
            .participants
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
            Seat::Player => self.handle_player_command(conn, id, body, wall),
            Seat::Gm => self.handle_gm_command(conn, id, body, wall),
        }
    }

    fn handle_player_command(
        &mut self,
        conn: ConnId,
        id: Option<String>,
        body: ClientBody,
        wall: &str,
    ) {
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
        let Some(actor_id) = self
            .participants
            .claim_of(conn)
            .map(|claim| claim.actor_id.clone())
        else {
            return self.reply(conn, id, ServerBody::Refused(Reason::ChoosePc.into()));
        };
        let presence = self.participants.presence();
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
                    Ok(view) => message_body::build_view(&view),
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
                Some(args) => {
                    let target = args.target;
                    return self.play_move(conn, id, &actor_id, &command, target, wall);
                }
                None => ServerBody::Refused(Reason::Schema.into()),
            },
        };
        self.reply(conn, id, body);
    }

    /// A claim takes a free PC, or by its token takes back one this player held. The reply
    /// is `claimed`, then a refresh.
    fn claim(&mut self, conn: ConnId, id: Option<String>, args: &Map<String, Value>) {
        let args = read::<ClaimArgs>(args).filter(|_| self.participants.claim_of(conn).is_none());
        let fresh = matches!(args, Some(ClaimArgs::Actor { .. }));
        let taken = match args {
            Some(ClaimArgs::Actor { actor_id }) => self.claim_free_pc(&actor_id),
            Some(ClaimArgs::Token { token, last_seq }) => self.claim_by_token(&token, last_seq),
            None => Err(Reason::Schema),
        };
        let (token, actor_id, claim_reply) = match taken {
            Ok(taken) => taken,
            Err(code) => return self.reply(conn, id, ServerBody::Refused(code.into())),
        };
        let [resume, view] = match refresh(&self.session, &self.participants.presence(), &actor_id)
        {
            Ok(bodies) => bodies,
            Err(code) => {
                if fresh {
                    self.participants
                        .claims
                        .retain(|claim| claim.token != token);
                }
                return self.reply(conn, id, ServerBody::Refused(code.into()));
            }
        };
        self.reply(conn, id, claim_reply);
        tracing::info!(conn_id = %conn, actor_id = %actor_id, by_token = !fresh, "PC claimed");
        if let Some(claim) = self.participants.claim_of_actor_mut(&actor_id) {
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
        let presence = self.participants.presence();
        if presence.actors_present.len() >= MAX_PLAYER_SEATS {
            return Err(Reason::TableFull);
        }
        let actor_id = self.session.find_pc(text)?.ok_or(Reason::Schema)?;
        if presence.actors_present.contains(&actor_id) {
            return Err(Reason::SeatTaken);
        }
        let scrollback = self.session.player_scrollback(&actor_id)?;
        let claim = Claim::new(uuid::Uuid::new_v4().simple().to_string(), actor_id.clone());
        let claim_reply =
            message_body::build_claim_reply(self.session.world(), &claim, &scrollback, 0);
        let token = claim.token.clone();
        self.participants.claims.push(claim);
        Ok((token, actor_id, claim_reply))
    }

    /// The held claim `token` names, with what was sent to it after `last_seq`. A connection
    /// that still drives it lets go.
    fn claim_by_token(
        &mut self,
        token: &str,
        last_seq: u64,
    ) -> Result<(String, ActorId, ServerBody), Reason> {
        let claim = self
            .participants
            .claims
            .iter_mut()
            .find(|claim| claim.token == token)
            .ok_or(Reason::UnknownId)?;
        claim.conn = None;
        let claim_reply =
            message_body::build_claim_reply(self.session.world(), claim, &[], last_seq);
        Ok((token.to_string(), claim.actor_id.clone(), claim_reply))
    }

    fn release_claim(&mut self, conn: ConnId, id: Option<String>) {
        if let Some(claim) = self.participants.claim_of(conn) {
            tracing::info!(conn_id = %conn, actor_id = %claim.actor_id, "PC released");
        }
        self.participants
            .claims
            .retain(|claim| claim.conn != Some(conn));
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
        let presence = self.participants.presence();
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
        wall: &str,
    ) {
        let gm = self.participants.gm;
        let presence = self.participants.presence();
        let target = target.filter(|target| !target.is_empty());
        let reply =
            match self
                .session
                .queue_player_move(&presence, actor_id, verb, target.as_deref())
            {
                Ok((queued, target)) => ServerBody::Queued {
                    intent: queued.id,
                    actor_id: queued.actor_id,
                    move_id: queued.move_id,
                    target,
                },
                Err(refused) if gm.is_none() && refused.code == Reason::Busy => {
                    ServerBody::Refused(message_body::build_refused_move(&refused))
                }
                Err(refused) => {
                    return self.reply(
                        conn,
                        id,
                        ServerBody::Refused(message_body::build_refused_move(&refused)),
                    );
                }
            };
        if let Some(gm) = gm {
            let seq = self.next_seq();
            self.send_to_pc(actor_id, stamp_message(seq, id, reply.clone()));
            return self.send(gm, stamp_message(seq, None, reply));
        }
        match self.session.commit_then_fast_forward(&presence, wall) {
            Ok(outputs) => {
                self.reply(conn, id, reply);
                self.after_rounds(&outputs);
            }
            Err(code) => self.reply(conn, id, ServerBody::Refused(code.into())),
        }
    }

    fn handle_gm_command(
        &mut self,
        conn: ConnId,
        id: Option<String>,
        body: ClientBody,
        wall: &str,
    ) {
        if self.participants.gm != Some(conn) {
            return self.reply(conn, id, ServerBody::Refused(Reason::NotInSpec.into()));
        }
        let presence = self.participants.presence();
        let reply = gm::run(&mut self.session, &presence, &body.kind, &body.args, wall)
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

    /// Frees the GM seat and says so on the console, so the operator sees the seat is open.
    fn release_gm(&mut self) {
        self.participants.gm = None;
        let _ = writeln!(self.out, "gm seat free");
        let _ = self.out.flush();
    }

    /// The one order after rounds: a `round` to every seat, then each player's checks,
    /// refresh and lines, then the digest, then the idle timer restarts.
    fn after_rounds(&mut self, outputs: &[crate::round::Output]) {
        if outputs.is_empty() {
            return;
        }
        let players: Vec<ActorId> = self
            .participants
            .claims
            .iter()
            .map(|claim| claim.actor_id.clone())
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
            if let Some(gm) = self.participants.gm {
                self.send(gm, round);
            }
        }
        let presence = self.participants.presence();
        for actor_id in players {
            let mut bodies: Vec<ServerBody> = outputs
                .iter()
                .flat_map(|output| message_body::build_checks(&output.reports, &actor_id))
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
        self.outbox.restart_idle = true;
    }

    /// The one speech path: the console's tape line, then one `say` with one `seq` to the GM and
    /// to each player who hears it. The sender's copy is its reply.
    fn deliver_speech(&mut self, speech: &TapeRecord, sender: Option<(ConnId, Option<String>)>) {
        let _ = writeln!(self.out, "{}", emit_tape_record(speech));
        let _ = self.out.flush();
        let seq = self.next_seq();
        let (sender, id) = sender.unzip();
        let id = id.flatten();
        let reply_to = |conn: ConnId| (sender == Some(conn)).then(|| id.clone()).flatten();
        let hears: Vec<(ActorId, Option<ConnId>)> = self
            .participants
            .claims
            .iter()
            .filter(|claim| {
                speech.recipient_ids.iter().any(|id| claim.actor_id == **id)
                    || claim.conn.is_some_and(|conn| sender == Some(conn))
            })
            .map(|claim| (claim.actor_id.clone(), claim.conn))
            .collect();
        if let Some(gm) = self.participants.gm {
            let body = ServerBody::Say(message_body::build_speech(speech, true));
            self.send(gm, stamp_message(seq, reply_to(gm), body));
        }
        for (actor_id, conn) in hears {
            let body = ServerBody::Say(message_body::build_speech(speech, false));
            let message = stamp_message(seq, conn.and_then(reply_to), body);
            self.send_to_pc(&actor_id, message);
        }
    }

    /// Tells the GM who is present now, and the console.
    fn announce_presence(&mut self) {
        if let Some(gm) = self.participants.gm {
            let seq = self.next_seq();
            let body = ServerBody::Presence {
                actors_present: self
                    .participants
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
        match self
            .participants
            .claim_of(conn)
            .map(|claim| claim.actor_id.clone())
        {
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
        let Some(claim) = self.participants.claim_of_actor_mut(actor_id) else {
            return;
        };
        claim.keep(&message);
        if let Some(conn) = claim.conn {
            self.send(conn, message);
        }
    }

    fn send(&mut self, conn: ConnId, message: ServerMessage) {
        self.outbox.messages.push((conn, message));
    }

    fn write_digest(&mut self, outputs: &[crate::round::Output]) {
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
    session: &Session,
    presence: &Presence,
    actor_id: &ActorId,
) -> Result<[ServerBody; 2], Reason> {
    let view = session.perceive(presence, actor_id.as_str())?;
    Ok([
        message_body::build_resume(&view),
        message_body::build_view(&view),
    ])
}
