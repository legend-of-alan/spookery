//! The game port's WebSocket endpoint: `/player` and `/gm`. One text message is one
//! `ClientMessage` in and one `ServerMessage` out. It frames messages and knows no
//! rules.

use std::io;
use std::sync::atomic::AtomicU64;
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::serve::ListenerExt;
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use spookery_engine::Reason;

use super::limits::{
    wait_for_token, TokenBucket, BURST, CLIENT_MESSAGE_CAP, KEEPALIVE, MESSAGE_CAP, RATE_PER_SEC,
    REQUESTED_SEND_BUFFER,
};
use super::seat::{frame_queue, write_frames, Conn};
use super::table::{report_close, Rejection, Request};
use crate::protocol::{ClientMessage, Seat};

#[derive(Clone)]
struct Acceptor {
    tx: Sender<Request>,
    next_conn: Arc<AtomicU64>,
    first_message_timeout: Duration,
}

/// Serve the endpoint until the listener fails.
pub(super) async fn serve(
    listener: tokio::net::TcpListener,
    tx: Sender<Request>,
    next_conn: Arc<AtomicU64>,
    first_message_timeout: Duration,
) {
    let acceptor = Acceptor {
        tx,
        next_conn,
        first_message_timeout,
    };
    let app = axum::Router::new()
        .route("/{seat}", axum::routing::get(upgrade))
        .with_state(acceptor);
    let listener = listener.tap_io(|stream| {
        let _ = set_socket_options(stream);
    });
    let _ = axum::serve(listener, app).await;
}

/// Sends small messages at once, drops a peer that stops answering keepalive probes, and
/// asks for a small send buffer so a client that stops reading fills its queue. The OS may
/// grant more, and the queue's byte cap still drops that connection.
fn set_socket_options(stream: &tokio::net::TcpStream) -> io::Result<()> {
    stream.set_nodelay(true)?;
    let socket = socket2::SockRef::from(stream);
    socket.set_tcp_keepalive(&KEEPALIVE)?;
    socket.set_send_buffer_size(REQUESTED_SEND_BUFFER)
}

async fn upgrade(
    State(acceptor): State<Acceptor>,
    Path(seat): Path<String>,
    headers: HeaderMap,
    socket: WebSocketUpgrade,
) -> Response {
    let seat = match seat.as_str() {
        "player" => Seat::Player,
        "gm" => Seat::Gm,
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    if !is_loopback(&headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    socket
        .max_message_size(MESSAGE_CAP)
        .max_frame_size(MESSAGE_CAP)
        .write_buffer_size(0)
        .on_upgrade(move |socket| run(socket, acceptor, seat))
}

/// The `Host` is `127.0.0.1`, and a browser's `Origin`, when it sends one, is too, so a page
/// on another site cannot reach the table through a renamed host.
fn is_loopback(headers: &HeaderMap) -> bool {
    let named = |value: Option<&str>, prefix: &str| {
        value
            .and_then(|value| value.strip_prefix(prefix))
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(':'))
    };
    let text = |name| headers.get(name).and_then(|value| value.to_str().ok());
    named(text(header::HOST), "127.0.0.1")
        && (headers.get(header::ORIGIN).is_none()
            || named(text(header::ORIGIN), "http://127.0.0.1"))
}

async fn run(socket: WebSocket, acceptor: Acceptor, seat: Seat) {
    let conn = Conn::new(&acceptor.next_conn);
    let (frames_tx, frames_rx) = frame_queue();
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    let open = Request::Open {
        conn,
        seat,
        frames_tx,
        reply: reply_tx,
    };
    if acceptor.tx.send(open).is_err() {
        return;
    }
    let Ok(opened) = reply_rx.await else {
        return;
    };
    let (mut sink, mut stream) = socket.split();
    if let Err(refused) = opened {
        let _ = sink.send(Message::text(refused.to_text())).await;
        let _ = sink.close().await;
        return;
    }
    let mut cancel_rx = frames_rx.cancel.clone();
    let writer = tokio::spawn(write_frames(
        async move |frame: &[u8]| match std::str::from_utf8(frame) {
            Ok(text) => sink.send(Message::text(text)).await.is_ok(),
            Err(_) => false,
        },
        frames_rx,
    ));
    let mut bucket = TokenBucket::new(RATE_PER_SEC, BURST);
    let mut spoke = false;
    loop {
        // Wait before reading so a full window slows the sender. The message is kept.
        if !wait_for_token(&mut bucket, &mut cancel_rx).await {
            break;
        }
        let first_message_deadline = async {
            if spoke {
                std::future::pending::<()>().await;
            }
            tokio::time::sleep(acceptor.first_message_timeout).await;
        };
        let received = tokio::select! {
            biased;
            changed = cancel_rx.changed() => {
                if changed.is_err() || *cancel_rx.borrow() {
                    break;
                }
                continue;
            }
            received = stream.next() => received,
            () = first_message_deadline => {
                report_close(&acceptor.tx, conn).await;
                break;
            }
        };
        let message = match received {
            Some(Ok(Message::Text(text))) => read_message(&text),
            Some(Ok(Message::Binary(_))) => Err(Rejection {
                id: None,
                code: Reason::Schema,
            }),
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
            Some(Ok(Message::Close(_)) | Err(_)) | None => {
                report_close(&acceptor.tx, conn).await;
                break;
            }
        };
        spoke = true;
        let _ = bucket.try_take(tokio::time::Instant::now());
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        let request = Request::Message {
            conn,
            message,
            reply: reply_tx,
        };
        // A connection the table dropped is cancelled, so the next wait ends the loop.
        if acceptor.tx.send(request).is_err() || reply_rx.await.is_err() {
            break;
        }
    }
    let _ = writer.await;
}

/// One client message, read by serde_json. One over `CLIENT_MESSAGE_CAP` is `LINE_TOO_LONG`,
/// and one that does not parse is `SCHEMA`, with the `id` it named when it named one.
fn read_message(text: &str) -> Result<ClientMessage, Rejection> {
    let id = || {
        let value: Value = serde_json::from_str(text).ok()?;
        Some(value.get("id")?.as_str()?.to_string())
    };
    if text.len() > CLIENT_MESSAGE_CAP {
        return Err(Rejection {
            id: None,
            code: Reason::LineTooLong,
        });
    }
    serde_json::from_str(text).map_err(|_| Rejection {
        id: id(),
        code: Reason::Schema,
    })
}

#[cfg(test)]
mod tests {
    use axum::http::{header, HeaderMap, HeaderValue};

    use super::{is_loopback, read_message};

    fn headers(host: &str, origin: Option<&str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_str(host).unwrap());
        if let Some(origin) = origin {
            headers.insert(header::ORIGIN, HeaderValue::from_str(origin).unwrap());
        }
        headers
    }

    #[test]
    fn only_a_loopback_host_and_origin_may_open_a_seat() {
        assert!(is_loopback(&headers("127.0.0.1:9737", None)));
        assert!(is_loopback(&headers(
            "127.0.0.1:9737",
            Some("http://127.0.0.1:8000")
        )));
        assert!(!is_loopback(&headers("localhost:9737", None)));
        assert!(!is_loopback(&headers("127.0.0.10:9737", None)));
        assert!(!is_loopback(&headers(
            "127.0.0.1:9737",
            Some("http://evil.example")
        )));
    }

    #[test]
    fn an_unreadable_message_keeps_the_id_it_named() {
        let named = read_message(r#"{"id":"c1","body":{"word":7}}"#).unwrap_err();
        assert_eq!(named.id.as_deref(), Some("c1"));
        assert!(read_message("not json").unwrap_err().id.is_none());
        assert!(read_message(r#"{"id":"c2","body":{"word":"wait"}}"#).is_ok());
    }
}
