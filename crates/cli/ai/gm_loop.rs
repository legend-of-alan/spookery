//! Sit the GM seat and turn model replies into tool calls.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use spookery_agent::{Agent, Input, Model, Step};

use crate::ai::{report_error, Error};
use crate::gm_connection::{self, Connection, Incoming};

#[derive(Default)]
pub struct TurnWatch {
    pub listening: AtomicBool,
    pub busy: AtomicBool,
    pub sit_seq: AtomicU64,
    pub high_water: AtomicU64,
}

pub async fn run_gm<M: Model>(
    addr: SocketAddr,
    door: Option<SocketAddr>,
    model: &mut M,
    watch: &TurnWatch,
) -> Result<(), Error> {
    let mut client = match door {
        Some(door) => Connection::connect_door(door).await,
        None => Connection::connect_default_door(addr).await,
    }
    .map_err(report_connection_error)?;
    let result = async {
        let mut agent = Agent::new();
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
                    watch.busy.store(step != Step::Idle, Ordering::Release);
                    watch.listening.store(true, Ordering::Release);
                }
            }
            match step {
                Step::CallTool { name, arguments } => {
                    if watch.listening.load(Ordering::Acquire) {
                        watch.busy.store(true, Ordering::Release);
                    }
                    let outcome = client
                        .call(name, arguments)
                        .await
                        .map_err(report_connection_error)?;
                    for event in client.take_events() {
                        agent
                            .feed(Input::GmEvent(event.body))
                            .map_err(report_error)?;
                    }
                    step = agent
                        .feed(Input::ToolResult {
                            body: outcome.body,
                            refused: outcome.refused,
                            raw: outcome.raw,
                        })
                        .map_err(report_error)?;
                }
                Step::CallModel { request, warning } => {
                    if let Some(warning) = warning {
                        report(&warning);
                    }
                    let reply = model.complete(&request).await;
                    step = agent.feed(Input::ModelReply(reply)).map_err(report_error)?;
                }
                Step::SendPlayerLine(_) => return Err(report_error("unexpected player line")),
                Step::Idle => {
                    watch.busy.store(false, Ordering::Release);
                    match client.next_incoming().await {
                        Incoming::Event(event) => {
                            step = agent
                                .feed(Input::GmEvent(event.body))
                                .map_err(report_error)?;
                            if step != Step::Idle && watch.listening.load(Ordering::Acquire) {
                                watch.busy.store(true, Ordering::Release);
                            }
                        }
                        Incoming::Result { raw, .. }
                        | Incoming::Invalid(raw)
                        | Incoming::Refused { raw, .. } => return Err(report_error(raw)),
                        Incoming::Eof if agent.sit_seq.is_some() => return Ok(()),
                        Incoming::Eof => return Err(report_error("seat closed")),
                    }
                }
            }
        }
    }
    .await;
    client.close().await;
    result
}

fn report(line: &str) {
    eprintln!("{line}");
    let _ = std::io::Write::flush(&mut std::io::stderr());
}

fn report_connection_error(err: gm_connection::Error) -> Error {
    match err {
        gm_connection::Error::Io(err) => report_error(err.to_string()),
        gm_connection::Error::Invalid(line) => report_error(line),
        gm_connection::Error::Refused { raw, .. } => report_error(raw),
    }
}
