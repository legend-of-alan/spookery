//! Sit the GM seat and turn model replies into tool calls.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use spookery_agent::{Agent, AgentInput, AgentStep, Model};

use crate::ai::{fail, Error};
use crate::gm_connection::{self, GmConnection, Incoming};

#[derive(Default)]
pub struct TurnWatch {
    pub listening: AtomicBool,
    pub busy: AtomicBool,
    pub sit_seq: AtomicU64,
    pub high_water: AtomicU64,
}

pub fn run_gm<M: Model>(
    _pack: &Path,
    addr: SocketAddr,
    door: Option<SocketAddr>,
    model: &mut M,
    watch: &TurnWatch,
) -> Result<(), Error> {
    let mut client = match door {
        Some(door) => GmConnection::connect_door(door),
        None => GmConnection::connect(addr),
    }
    .map_err(from_connection_err)?;
    let mut agent = Agent::new_gm();
    let mut step = agent.start();
    loop {
        if let Some(warning) = agent.warning.take() {
            report(&warning);
        }
        if let Some(sit) = agent.sit_seq {
            watch.high_water.store(agent.high_water, Ordering::Release);
            if !watch.listening.load(Ordering::Acquire) {
                report("listening");
                watch.sit_seq.store(sit, Ordering::Release);
                watch.high_water.store(sit, Ordering::Release);
                watch.busy.store(step != AgentStep::Idle, Ordering::Release);
                watch.listening.store(true, Ordering::Release);
            }
        }
        match step {
            AgentStep::CallTool { name, arguments } => {
                if watch.listening.load(Ordering::Acquire) {
                    watch.busy.store(true, Ordering::Release);
                }
                let outcome = client.call(name, arguments).map_err(from_connection_err)?;
                for event in client.take_events() {
                    agent.feed(AgentInput::GmEvent(event.body)).map_err(fail)?;
                }
                step = agent
                    .feed(AgentInput::ToolResult {
                        body: outcome.body,
                        refused: outcome.refused,
                        raw: outcome.raw,
                    })
                    .map_err(fail)?;
            }
            AgentStep::CallModel { request, warning } => {
                if let Some(warning) = warning {
                    report(&warning);
                }
                let reply = model.complete(&request);
                step = agent.feed(AgentInput::ModelReply(reply)).map_err(fail)?;
            }
            AgentStep::SendPlayerLine(_) => return Err(fail("unexpected player line")),
            AgentStep::Idle => {
                watch.busy.store(false, Ordering::Release);
                match client.next_incoming() {
                    Incoming::Event(event) => {
                        step = agent.feed(AgentInput::GmEvent(event.body)).map_err(fail)?;
                        if step != AgentStep::Idle && watch.listening.load(Ordering::Acquire) {
                            watch.busy.store(true, Ordering::Release);
                        }
                    }
                    Incoming::Result { raw, .. }
                    | Incoming::Bad(raw)
                    | Incoming::Refused { raw, .. } => return Err(fail(raw)),
                    Incoming::Eof if agent.sit_seq.is_some() => return Ok(()),
                    Incoming::Eof => return Err(fail("seat closed")),
                }
            }
        }
    }
}

fn report(line: &str) {
    eprintln!("{line}");
    let _ = std::io::Write::flush(&mut std::io::stderr());
}

fn from_connection_err(err: gm_connection::Error) -> Error {
    match err {
        gm_connection::Error::Io(err) => fail(err.to_string()),
        gm_connection::Error::Closed => fail("seat closed"),
        gm_connection::Error::Bad(line) => fail(line),
        gm_connection::Error::Refused { raw, .. } => fail(raw),
    }
}
