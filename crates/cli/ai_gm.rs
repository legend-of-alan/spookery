//! Sit the GM seat and turn model replies into words.

use std::io::Write;
use std::net::SocketAddr;

use spookery_agent::{Agent, Input, Model, Step};

use crate::ws::Connection;
use crate::Error;

/// Sit the GM seat on the game port at `addr` and play each turn through `model` until the
/// host closes the seat. `listening`, warnings and model-call errors go to `err`, one line
/// each.
pub async fn run(
    addr: SocketAddr,
    model: &mut impl Model,
    err: &mut (impl Write + Send),
) -> Result<(), Error> {
    let (mut seat, _hello) = Connection::open(addr, "gm").await?;
    let mut agent = Agent::new();
    let mut step = agent.start();
    let mut listening = false;
    loop {
        if let Some(warning) = agent.take_warning() {
            report(err, &warning)?;
        }
        step = match step {
            Step::Idle => match seat.recv().await {
                Some(Ok(message)) => {
                    let replied = message.reply_to.is_some();
                    let step = agent.feed(Input::Message(message))?;
                    // The first reply is the sit's status: the agent is seated once it is read.
                    if replied && !listening {
                        report(err, "listening")?;
                        listening = true;
                    }
                    step
                }
                Some(Err(err)) => return Err(err),
                None => return Ok(()),
            },
            Step::Send(message) => {
                seat.send_message(&message).await?;
                Step::Idle
            }
            Step::CallModel(request) => {
                let reply = model.complete(&request).await;
                agent.feed(Input::ModelReply(reply))?
            }
        };
    }
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
