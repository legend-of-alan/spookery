//! The client side of the GM seat over the agent door.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::Arc;

use rmcp::model::{
    CallToolRequestParams, ClientCapabilities, ClientConfig, ClientJsonRpcMessage, ClientRequest,
    CustomNotification, ErrorData, Implementation, InitializeRequest, JsonRpcError,
    ProtocolVersion, RequestId,
};
use rmcp::service::{ClientInitializeError, NotificationContext, RoleClient, RunningService};
use rmcp::transport::common::client_side_sse::NeverRetry;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::{StreamableHttpClientTransport, Transport};
use rmcp::{ClientHandler, ServiceExt};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;

use crate::protocol::{
    self, CommitArgs, CommitBody, GmEvent, IntentCall, PerceiveArgs, Queued, Refusal, SayArgs,
    SayEvent, StatusArgs, TableStatus,
};
use crate::Error;

/// Sent from the agent door to a GM client between its tool calls: an event, an event the
/// client could not read (as its JSON-RPC line), or the end of the seat.
#[derive(Clone, Debug, PartialEq)]
pub enum Incoming {
    Event(GmEvent),
    Unreadable(String),
    Eof,
}

struct Inbox {
    incoming: mpsc::UnboundedSender<Incoming>,
}

impl ClientHandler for Inbox {
    fn on_custom_notification(
        &self,
        notification: CustomNotification,
        _context: NotificationContext<RoleClient>,
    ) -> impl std::future::Future<Output = ()> + Send + '_ {
        if notification.method == "spook/event" {
            let _ = self.incoming.send(read_event(notification.params));
        }
        std::future::ready(())
    }

    fn get_info(&self) -> ClientConfig {
        ClientConfig::new(
            ClientCapabilities::default(),
            Implementation::new("spook-gm", "0"),
        )
        .with_protocol_version(ProtocolVersion::LATEST)
    }
}

/// The client side of the GM seat: dials the agent door, calls the six tools, hears events.
pub struct GmClient {
    service: RunningService<RoleClient, Inbox>,
    /// Finishes when the host closes the agent door.
    watch: tokio::task::JoinHandle<()>,
    incoming: mpsc::UnboundedReceiver<Incoming>,
    buffered: VecDeque<Incoming>,
    protocol_version: String,
}

impl GmClient {
    /// Dial `http://<door>/mcp` and finish the handshake. `door` is the agent door, not the
    /// game port.
    pub async fn connect(door: SocketAddr) -> Result<Self, Error> {
        let url = format!("http://{door}/mcp");
        let (events, incoming) = mpsc::unbounded_channel();
        // A closed loopback seat ends the stream. The default client retries
        // that stream forever, so a dropped `spook run` never looks closed.
        let mut config = StreamableHttpClientTransportConfig::with_uri(url.clone());
        config.retry_config = Arc::new(NeverRetry::default());
        config.reinit_on_expired_session = false;
        let transport = ClientTransport {
            inner: StreamableHttpClientTransport::from_config(config),
            incoming: events.clone(),
            closed: false,
        };
        let inbox = Inbox {
            incoming: events.clone(),
        };
        let info = inbox.get_info();
        let service = match inbox.serve(transport).await {
            Ok(service) => service,
            // The streamable client skips a JSON-RPC error on the initialize
            // event stream and reports the stream as closed. Read that refusal
            // from a fresh response body.
            Err(err @ ClientInitializeError::ConnectionClosed(_)) => {
                return Err(match probe_seat_refusal(&url, info).await {
                    Some(refusal) => Error::Refused(refusal),
                    None => err.into(),
                });
            }
            Err(err) => return Err(err.into()),
        };
        // rmcp keeps no read on the agent port once a tool call's stream
        // ends, so a killed `spook run` never looks closed. A plain connection
        // with a pending read sees that close the way the TCP client did.
        let watch = tokio::spawn(async move {
            if let Ok(mut socket) = tokio::net::TcpStream::connect(door).await {
                let mut buf = [0u8; 1];
                tokio::select! {
                    _ = tokio::io::AsyncReadExt::read(&mut socket, &mut buf) => {}
                    () = events.closed() => return,
                }
            }
            let _ = events.send(Incoming::Eof);
        });
        let protocol_version = service
            .peer_info()
            .map(|info| info.protocol_version.as_str().to_string())
            .unwrap_or_default();
        Ok(GmClient {
            service,
            watch,
            incoming,
            buffered: VecDeque::new(),
            protocol_version,
        })
    }

    /// The protocol version the door answered the initialize with.
    pub fn protocol_version(&self) -> &str {
        &self.protocol_version
    }

    /// Who is seated, the queue, and (without `since`) the pack summary. With `since`, the
    /// say events after that tape sequence too.
    pub async fn table_status(
        &mut self,
        since: Option<u64>,
    ) -> Result<Result<TableStatus, Refusal>, Error> {
        self.call("table_status", &StatusArgs { since }).await
    }

    /// The view of the actor `viewer`, a bare actor id.
    pub async fn perceive(
        &mut self,
        viewer: &str,
    ) -> Result<Result<protocol::View, Refusal>, Error> {
        let args = PerceiveArgs {
            as_viewer: Some(viewer.to_string()),
            max_body_chars: None,
        };
        self.call("perceive", &args).await
    }

    /// Queue one move. The table answers with the intent's id.
    pub async fn queue_intent(
        &mut self,
        call: &IntentCall,
    ) -> Result<Result<Queued, Refusal>, Error> {
        self.call("queue_intent", call).await
    }

    /// Speak as the GM. The table answers with the say it put on the tape.
    pub async fn say(&mut self, args: &SayArgs) -> Result<Result<SayEvent, Refusal>, Error> {
        self.call("say", args).await
    }

    /// Commit the queue as one round.
    pub async fn commit_round(&mut self) -> Result<Result<CommitBody, Refusal>, Error> {
        let args = CommitArgs {
            include_round: true,
            session_id: None,
        };
        self.call("commit_round", &args).await
    }

    /// Call the tool `name` and read its structured content as `T` or as a refusal. The
    /// methods above are this call with each tool's own argument and reply types. A reply
    /// that is neither is `Error::Unreadable`.
    pub async fn call<T: DeserializeOwned>(
        &mut self,
        name: &str,
        args: &impl Serialize,
    ) -> Result<Result<T, Refusal>, Error> {
        let arguments = match serde_json::to_value(args) {
            Ok(Value::Object(arguments)) => arguments,
            other => return Err(Error::Message(format!("{name} arguments: {other:?}"))),
        };
        let params = CallToolRequestParams::new(name.to_string()).with_arguments(arguments);
        let result = self
            .service
            .call_tool(params)
            .await
            .map_err(|err| Error::Message(err.to_string()))?;
        while let Ok(item) = self.incoming.try_recv() {
            self.buffered.push_back(item);
        }
        let body = result.structured_content.unwrap_or(Value::Null);
        if let Ok(refusal) = Refusal::deserialize(&body) {
            return Ok(Err(refusal));
        }
        T::deserialize(&body)
            .map(Ok)
            .map_err(|_| Error::Unreadable(body.to_string()))
    }

    /// Events read while waiting for the latest tool result, in arrival order.
    pub fn take_events(&mut self) -> Vec<GmEvent> {
        let mut out = Vec::new();
        while matches!(self.buffered.front(), Some(Incoming::Event(_))) {
            if let Some(Incoming::Event(event)) = self.buffered.pop_front() {
                out.push(event);
            }
        }
        out
    }

    pub async fn next_incoming(&mut self) -> Incoming {
        if let Some(item) = self.buffered.pop_front() {
            return item;
        }
        self.incoming.recv().await.unwrap_or(Incoming::Eof)
    }

    /// Buffered lines, then anything the reader has already queued.
    pub fn drain_ready(&mut self) -> Vec<Incoming> {
        let mut out: Vec<Incoming> = self.buffered.drain(..).collect();
        while let Ok(item) = self.incoming.try_recv() {
            out.push(item);
        }
        out
    }

    /// Give the seat back. The host has freed it when this returns.
    pub async fn close(self) {
        // A closed seat has nothing to end. Cancel would send the session
        // DELETE to the dead port, and Windows takes about 2s to refuse it.
        if !self.watch.is_finished() {
            let _ = self.service.cancel().await;
        }
    }
}

/// The client side of the streamable HTTP transport, plus one end-of-seat notice.
///
/// `receive` returning nothing is the seat going away. The TCP client turned
/// that into `Incoming::Eof`. Callers waiting on the next event use the same
/// signal to leave.
struct ClientTransport<T> {
    inner: T,
    incoming: mpsc::UnboundedSender<Incoming>,
    closed: bool,
}

impl<T> Transport<RoleClient> for ClientTransport<T>
where
    T: Transport<RoleClient>,
{
    type Error = T::Error;

    fn send(
        &mut self,
        item: rmcp::service::TxJsonRpcMessage<RoleClient>,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send + 'static {
        self.inner.send(item)
    }

    async fn receive(&mut self) -> Option<rmcp::service::RxJsonRpcMessage<RoleClient>> {
        let message = self.inner.receive().await;
        if message.is_none() && !self.closed {
            self.closed = true;
            let _ = self.incoming.send(Incoming::Eof);
        }
        message
    }

    fn close(&mut self) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send {
        self.inner.close()
    }
}

/// Send the session's own initialize, `info`, again on a plain request and read the
/// refusal from the `data` of the JSON-RPC error in its event stream.
async fn probe_seat_refusal(url: &str, info: ClientConfig) -> Option<Refusal> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .ok()?;
    // This initialize can sit a seat that freed after the first attempt.
    // The DELETE below gives that seat back.
    let version = info.protocol_version.as_str().to_string();
    let initialize = ClientJsonRpcMessage::request(
        ClientRequest::InitializeRequest(InitializeRequest::new(info)),
        RequestId::Number(1),
    );
    let response = client
        .post(url)
        .header("accept", "application/json, text/event-stream")
        .header("content-type", "application/json")
        .header("mcp-protocol-version", version)
        .body(serde_json::to_string(&initialize).ok()?)
        .send()
        .await
        .ok()?;
    let session = response
        .headers()
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let text = response.text().await.ok()?;
    if let Some(session) = session {
        let _ = client
            .delete(url)
            .header("mcp-session-id", session)
            .send()
            .await;
    }
    text.lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .find_map(|data| serde_json::from_str::<JsonRpcError>(data.trim()).ok())
        .and_then(|reply| refusal_in(&reply.error))
}

impl From<ClientInitializeError> for Error {
    fn from(err: ClientInitializeError) -> Self {
        match refusal_of(&err) {
            Some(refusal) => Error::Refused(refusal),
            None => Error::Io(std::io::Error::other(err.to_string())),
        }
    }
}

fn refusal_of(err: &ClientInitializeError) -> Option<Refusal> {
    match err {
        ClientInitializeError::JsonRpcError(error) => refusal_in(error),
        ClientInitializeError::LegacyFallbackFailed { discover, fallback } => {
            refusal_of(discover).or_else(|| refusal_of(fallback))
        }
        _ => None,
    }
}

/// The refusal the table puts in an MCP error's `data`.
fn refusal_in(error: &ErrorData) -> Option<Refusal> {
    Refusal::deserialize(error.data.as_ref()?).ok()
}

/// Read one `spook/event` notification. A body that is not a say, a queued move, or a
/// check is `Incoming::Unreadable`.
fn read_event(params: Option<Value>) -> Incoming {
    let params = params.unwrap_or(Value::Null);
    match GmEvent::deserialize(&params) {
        Ok(event) => Incoming::Event(event),
        Err(_) => Incoming::Unreadable(event_line(&params)),
    }
}

/// One `spook/event` notification as its JSON-RPC line.
pub(crate) fn event_line(params: &impl Serialize) -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "method": "spook/event",
        "params": params,
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_body_that_matches_no_known_type_is_unreadable() {
        let Incoming::Unreadable(line) = read_event(Some(serde_json::json!({"kind": "nope"})))
        else {
            panic!("malformed event was dropped");
        };
        assert!(line.contains("\"kind\":\"nope\""), "{line}");
    }
}
