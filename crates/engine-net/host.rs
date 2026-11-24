//! The host thread: it feeds each request from the game port, the MCP endpoint and the idle timer
//! to the router, writes what the router sends to each connection's queue, and answers the
//! request.

use std::sync::mpsc::{Receiver, Sender};

use spookery_engine::transport::{self, ConnId, Outbox, Rejection};
use spookery_engine::Reason;
use tokio::sync::{oneshot, watch};

use super::idle::{self, now_stamp};
use super::outbound::FrameSender;
use super::Error;
use crate::protocol::{ClientMessage, Seat, ServerMessage};

/// Sent from the game port, the MCP endpoint and the idle timer to the host thread.
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
    /// A connection closed. The host lets go of it, then acks.
    Close {
        conn: ConnId,
        ack: oneshot::Sender<()>,
    },
    /// One MCP tool call, as a client message. The reply is every message the host sent
    /// this connection while it ran the command, the command's own reply last.
    ToolCall {
        conn: ConnId,
        message: ClientMessage,
        reply: oneshot::Sender<Vec<ServerMessage>>,
    },
    Wake,
    Stop,
}

/// Tells the host a connection closed, and waits until the host has let go of it.
pub(super) async fn report_close(tx: &Sender<Request>, conn: ConnId) {
    let (ack_tx, ack_rx) = oneshot::channel();
    if tx.send(Request::Close { conn, ack: ack_tx }).is_err() {
        return;
    }
    let _ = ack_rx.await;
}

pub(super) struct Host {
    router: transport::Router,
    /// Each open connection's outbound queue. Dropping one cancels its socket.
    frame_senders: Vec<(ConnId, FrameSender)>,
    /// Bumped when the router played rounds, which restarts the idle timer.
    idle: watch::Sender<u64>,
}

impl Host {
    pub(super) fn new(router: transport::Router, idle: watch::Sender<u64>) -> Self {
        Self {
            router,
            frame_senders: Vec::new(),
            idle,
        }
    }

    /// Answer requests until `Stop`. A store fault ends the host.
    pub(super) fn run(mut self, rx: Receiver<Request>) -> Result<(), Error> {
        loop {
            match rx.recv().map_err(|_| Error::Reason(Reason::NotInSpec))? {
                Request::Open {
                    conn,
                    seat,
                    frames_tx,
                    reply,
                } => match self.router.open(conn, seat) {
                    Ok(outbox) => {
                        self.frame_senders.push((conn, frames_tx));
                        self.enqueue(outbox);
                        if reply.send(Ok(())).is_err() {
                            self.close(conn);
                        }
                    }
                    Err(refused) => {
                        let _ = reply.send(Err(refused));
                    }
                },
                Request::Message {
                    conn,
                    message,
                    reply,
                } => {
                    let outbox = self.router.receive(conn, message, &now_stamp());
                    self.enqueue(outbox);
                    let _ = reply.send(());
                }
                Request::Close { conn, ack } => {
                    self.close(conn);
                    let _ = ack.send(());
                }
                Request::ToolCall {
                    conn,
                    message,
                    reply,
                } => {
                    let outbox = self.router.call_tool(conn, message, &now_stamp());
                    let (to_caller, to_others): (Vec<_>, _) =
                        outbox.messages.into_iter().partition(|(to, _)| *to == conn);
                    self.enqueue(Outbox {
                        messages: to_others,
                        restart_idle: outbox.restart_idle,
                    });
                    let _ = reply.send(to_caller.into_iter().map(|(_, message)| message).collect());
                }
                Request::Wake => {
                    let outbox = self.router.wake(idle::catch_up_if_due);
                    self.enqueue(outbox.map_err(Error::Reason)?);
                }
                Request::Stop => return Ok(()),
            }
        }
    }

    /// Queues each message the router sent. A full or closed queue drops that connection. The
    /// host does not wait.
    fn enqueue(&mut self, outbox: Outbox) {
        if outbox.restart_idle {
            self.idle
                .send_modify(|count| *count = count.wrapping_add(1));
        }
        for (conn, message) in outbox.messages {
            let Some((_, frames_tx)) = self.frame_senders.iter().find(|(id, _)| *id == conn) else {
                continue;
            };
            if !frames_tx.send(message.to_text().into_bytes()) {
                tracing::debug!(conn_id = %conn, "outbound queue full or closed");
                self.close(conn);
            }
        }
    }

    /// Lets go of the connection in the router and drops its queue, which cancels its socket.
    fn close(&mut self, conn: ConnId) {
        self.router.close(conn);
        self.frame_senders.retain(|(id, _)| *id != conn);
    }
}
