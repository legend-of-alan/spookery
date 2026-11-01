//! Sit the GM seat and turn model replies into tool calls.

use std::io::Write;
use std::net::SocketAddr;

use spookery_agent::{Agent, Input, Model, Step};

use crate::mcp::GmClient;
use crate::Error;

/// Sit the GM seat at `mcp_addr` and play each turn through `model` until the host closes the
/// seat. `listening`, warnings and model-call errors go to `err`, one line each.
pub async fn run(
    mcp_addr: SocketAddr,
    model: &mut impl Model,
    err: &mut (impl Write + Send),
) -> Result<(), Error> {
    let mut client = GmClient::connect(mcp_addr).await?;
    let result = async {
        let mut agent = Agent::new();
        let mut step = agent.start();
        let mut listening = false;
        loop {
            if let Some(warning) = agent.take_warning() {
                report(err, &warning)?;
            }
            let reply = match step {
                Step::Idle => match client.recv().await {
                    Some(Ok(event)) => {
                        step = agent.feed(Input::Event(event))?;
                        continue;
                    }
                    Some(Err(err)) => return Err(err),
                    None => return Ok(()),
                },
                Step::CallModel(request) => {
                    let reply = model.complete(&request).await;
                    step = agent.feed(Input::ModelReply(reply))?;
                    continue;
                }
                Step::TableStatus { since } => {
                    client.table_status(since).await.map(Input::StatusReply)
                }
                Step::Perceive { viewer } => client.perceive(&viewer).await.map(Input::ViewReply),
                Step::Say(args) => client.say(&args).await.map(Input::SayReply),
                Step::QueueIntent(call) => client.queue_intent(&call).await.map(Input::QueueReply),
                Step::CommitRound => client.commit_round().await.map(Input::CommitReply),
            };
            let reply = reply?;
            for event in client.drain_events() {
                agent.feed(Input::Event(event))?;
            }
            step = agent.feed(reply)?;
            // The first reply is the sit's table_status: the agent is seated once it is read.
            if !listening {
                report(err, "listening")?;
                listening = true;
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

fn report(err: &mut impl Write, line: &str) -> Result<(), Error> {
    writeln!(err, "{line}")
        .and_then(|()| err.flush())
        .map_err(Error::Io)
}
