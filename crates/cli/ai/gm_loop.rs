//! Sit the GM seat and turn model replies into tool calls.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use serde_json::Value;
use spookery_agent::{Agent, Input, Model, Step};

use crate::ai::{report_error, Error};
use crate::gm_connection::{GmClient, Incoming};
use crate::protocol;

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
    let door = door
        .or_else(|| protocol::default_door(addr))
        .ok_or_else(|| report_error(addr.to_string()))?;
    let mut client = GmClient::connect(door)
        .await
        .map_err(|err| report_error(err.to_string()))?;
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
                    // The agent still asks for tools by name and reads replies as JSON.
                    let reply = client
                        .call::<Value>(name, &arguments)
                        .await
                        .map_err(|err| report_error(err.to_string()))?;
                    for event in client.take_events() {
                        agent.feed(Input::GmEvent(event)).map_err(report_error)?;
                    }
                    let refused = reply.is_err();
                    let body = reply.unwrap_or_else(|refusal| {
                        serde_json::to_value(refusal).expect("a refusal serializes")
                    });
                    let raw = body.to_string();
                    step = agent
                        .feed(Input::ToolResult { body, refused, raw })
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
                            step = agent.feed(Input::GmEvent(event)).map_err(report_error)?;
                            if step != Step::Idle && watch.listening.load(Ordering::Acquire) {
                                watch.busy.store(true, Ordering::Release);
                            }
                        }
                        Incoming::Unreadable(line) => return Err(report_error(line)),
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
