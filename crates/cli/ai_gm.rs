//! Sit the GM seat and turn model replies into tool calls.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};

use spookery_agent::{Agent, Input, Model, Step};

use crate::door::{GmClient, Incoming};
use crate::Error;

#[derive(Default)]
pub struct TurnWatch {
    pub listening: AtomicBool,
    pub busy: AtomicBool,
}

pub async fn run<M: Model>(
    door: SocketAddr,
    model: &mut M,
    watch: &TurnWatch,
) -> Result<(), Error> {
    let mut client = GmClient::connect(door).await?;
    let result = async {
        let mut agent = Agent::new();
        let mut step = agent.start();
        loop {
            if let Some(warning) = agent.take_warning() {
                report(&warning);
            }
            let reply = match step {
                Step::Idle => {
                    watch.busy.store(false, Ordering::Release);
                    match client.next_incoming().await {
                        Incoming::Event(event) => {
                            step = agent.feed(Input::Event(event))?;
                            if step != Step::Idle && watch.listening.load(Ordering::Acquire) {
                                watch.busy.store(true, Ordering::Release);
                            }
                            continue;
                        }
                        Incoming::Unreadable(line) => return Err(Error::Message(line)),
                        Incoming::Eof => return Ok(()),
                    }
                }
                Step::CallModel(request) => {
                    let reply = model.complete(&request).await;
                    step = agent.feed(Input::ModelReply(reply))?;
                    continue;
                }
                Step::FetchStatus { since } => {
                    client.table_status(since).await.map(Input::StatusReply)
                }
                Step::Perceive { viewer } => client.perceive(&viewer).await.map(Input::ViewReply),
                Step::Say(args) => client.say(&args).await.map(Input::SayReply),
                Step::QueueIntent(call) => client.queue_intent(&call).await.map(Input::QueueReply),
                Step::CommitRound => client.commit_round().await.map(Input::CommitReply),
            };
            let reply = reply?;
            for event in client.take_events() {
                agent.feed(Input::Event(event))?;
            }
            step = agent.feed(reply)?;
            // The first reply is the sit's table_status: the agent is seated once it is read.
            if !watch.listening.load(Ordering::Acquire) {
                report("listening");
                watch.busy.store(step != Step::Idle, Ordering::Release);
                watch.listening.store(true, Ordering::Release);
            }
        }
    }
    .await;
    client.close().await;
    result
}

impl From<spookery_agent::Error> for Error {
    fn from(err: spookery_agent::Error) -> Self {
        Error::Message(err.to_string())
    }
}

fn report(line: &str) {
    eprintln!("{line}");
    let _ = std::io::Write::flush(&mut std::io::stderr());
}
