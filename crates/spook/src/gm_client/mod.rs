use serde_json::Value;
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// A GM-seat failure. The display text is the socket line or the I/O error.
#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Closed,
    Bad(String),
    Refused { code: String, raw: String },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(err) => write!(f, "{err}"),
            Error::Closed => f.write_str("seat closed"),
            Error::Bad(line) => f.write_str(line),
            Error::Refused { raw, .. } => f.write_str(raw),
        }
    }
}

impl std::error::Error for Error {}

/// One `spook/event` the seat already sent. `raw` is the socket line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub raw: String,
    pub body: EventBody,
}

/// Fields the seat put on the event. A check has no actor and no check id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EventBody {
    Say {
        from: String,
        seq: u64,
        address: String,
        place: Option<String>,
        actor: Option<String>,
        text: String,
    },
    Queued {
        id: String,
        actor_id: String,
        move_name: String,
    },
    Check {
        faces: Vec<u64>,
        stat: i64,
        total: i64,
        target: i64,
        success: bool,
    },
}

/// A line from the GM socket, before a client's own turn rules.
#[derive(Clone, Debug, PartialEq)]
pub enum Incoming {
    Result { id: u64, raw: String },
    Event(Event),
    Bad(String),
    Refused { code: String, raw: String },
    Eof,
}

/// The structured body of one tool result.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolResult {
    pub raw: String,
    pub body: Value,
    pub refused: bool,
}

/// The loopback GM seat. Std threads and a `TcpStream`. No model client.
pub struct Client {
    write: TcpStream,
    incoming: mpsc::Receiver<Incoming>,
    buffered: VecDeque<Incoming>,
    next_id: u64,
    opened: String,
}

impl Client {
    /// `gm`, initialize, and `notifications/initialized`, then the initialize result.
    pub fn connect(addr: &str) -> Result<Self, Error> {
        let mut write = TcpStream::connect(addr).map_err(Error::Io)?;
        write.set_nodelay(true).map_err(Error::Io)?;
        let read = write.try_clone().map_err(Error::Io)?;
        let hello = format!(
            "{}{}{}",
            crate::protocol::GM_HELLO,
            crate::protocol::INITIALIZE_LINE,
            crate::protocol::INITIALIZED_LINE
        );
        write.write_all(hello.as_bytes()).map_err(Error::Io)?;
        write.flush().map_err(Error::Io)?;
        let (tx, incoming) = mpsc::channel();
        thread::spawn(move || read_loop(read, tx));
        let mut client = Client {
            write,
            incoming,
            buffered: VecDeque::new(),
            next_id: 2,
            opened: String::new(),
        };
        match client.recv_channel() {
            Incoming::Result { id: 1, raw } => {
                let outcome = parse_tool(&raw)?;
                if outcome.refused {
                    return Err(Error::Bad(raw));
                }
                client.opened = raw;
                Ok(client)
            }
            Incoming::Refused { code, raw } => Err(Error::Refused { code, raw }),
            Incoming::Bad(line) => Err(Error::Bad(line)),
            Incoming::Eof => Err(Error::Closed),
            Incoming::Result { raw, .. } => Err(Error::Bad(raw)),
            Incoming::Event(event) => Err(Error::Bad(event.raw)),
        }
    }

    /// The initialize result line, without a trailing newline.
    pub fn opened_line(&self) -> &str {
        &self.opened
    }

    pub fn call(&mut self, name: &str, arguments: Value) -> Result<ToolResult, Error> {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        let line = crate::protocol::write_tool_call(&crate::protocol::ToolCall {
            id,
            name: name.to_string(),
            arguments,
        });
        self.write.write_all(line.as_bytes()).map_err(Error::Io)?;
        self.write.flush().map_err(Error::Io)?;
        loop {
            match self.recv_channel() {
                Incoming::Result { id: got, raw } if got == id => return parse_tool(&raw),
                Incoming::Result { raw, .. } => return Err(Error::Bad(raw)),
                Incoming::Event(event) => self.buffered.push_back(Incoming::Event(event)),
                Incoming::Bad(line) => return Err(Error::Bad(line)),
                Incoming::Refused { code, raw } => return Err(Error::Refused { code, raw }),
                Incoming::Eof => return Err(Error::Closed),
            }
        }
    }

    /// Events read while waiting for the latest tool result, in socket order.
    pub fn take_events(&mut self) -> Vec<Event> {
        let mut out = Vec::new();
        while matches!(self.buffered.front(), Some(Incoming::Event(_))) {
            if let Some(Incoming::Event(event)) = self.buffered.pop_front() {
                out.push(event);
            }
        }
        out
    }

    pub fn next_incoming(&mut self) -> Incoming {
        if let Some(item) = self.buffered.pop_front() {
            return item;
        }
        self.recv_channel()
    }

    pub fn poll(&mut self, timeout: Duration) -> Option<Incoming> {
        if let Some(item) = self.buffered.pop_front() {
            return Some(item);
        }
        match self.incoming.recv_timeout(timeout) {
            Ok(item) => Some(item),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => Some(Incoming::Eof),
        }
    }

    /// Buffered lines, then anything the reader has already queued.
    pub fn drain_ready(&mut self) -> Vec<Incoming> {
        let mut out: Vec<Incoming> = self.buffered.drain(..).collect();
        while let Ok(item) = self.incoming.try_recv() {
            out.push(item);
        }
        out
    }

    fn recv_channel(&mut self) -> Incoming {
        self.incoming.recv().unwrap_or(Incoming::Eof)
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.write.shutdown(Shutdown::Both);
    }
}

fn parse_tool(raw: &str) -> Result<ToolResult, Error> {
    let body = crate::protocol::read_tool_result(raw).map_err(|_| Error::Bad(raw.to_string()))?;
    let refused = body.get("code").is_some();
    Ok(ToolResult {
        raw: raw.to_string(),
        body,
        refused,
    })
}

fn read_loop(mut read: TcpStream, tx: mpsc::Sender<Incoming>) {
    loop {
        let line = match read_line(&mut read) {
            Ok(Some(line)) => line,
            Ok(None) | Err(_) => {
                let _ = tx.send(Incoming::Eof);
                return;
            }
        };
        if let Some(incoming) = classify_line(&line) {
            if tx.send(incoming).is_err() {
                return;
            }
        }
    }
}

fn classify_line(line: &str) -> Option<Incoming> {
    let value: Value = match serde_json::from_str(line) {
        Ok(value) => value,
        Err(_) => return Some(Incoming::Bad(line.to_string())),
    };
    if value.get("kind").and_then(Value::as_str) == Some("refused") {
        let code = value
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        return Some(Incoming::Refused {
            code,
            raw: line.to_string(),
        });
    }
    if let Some(id) = value.get("id").and_then(Value::as_u64) {
        return Some(Incoming::Result {
            id,
            raw: line.to_string(),
        });
    }
    if value.get("method").and_then(Value::as_str) != Some("spook/event") {
        return Some(Incoming::Bad(line.to_string()));
    }
    let Some(params) = value.get("params") else {
        return Some(Incoming::Bad(line.to_string()));
    };
    let kind = params.get("kind").and_then(Value::as_str).unwrap_or("");
    match kind {
        "say" => Some(Incoming::Event(Event {
            raw: line.to_string(),
            body: EventBody::Say {
                from: text_field(params, "from"),
                seq: params.get("seq").and_then(Value::as_u64).unwrap_or(0),
                address: text_field(params, "address"),
                place: nonempty(params.get("place")),
                actor: nonempty(params.get("actor")),
                text: text_field(params, "text"),
            },
        })),
        "queued" => Some(Incoming::Event(Event {
            raw: line.to_string(),
            body: EventBody::Queued {
                id: text_field(params, "id"),
                actor_id: text_field(params, "actor_id"),
                move_name: text_field(params, "move"),
            },
        })),
        "check" => parse_check(params).map(|body| {
            Incoming::Event(Event {
                raw: line.to_string(),
                body,
            })
        }),
        _ => None,
    }
}

fn parse_check(params: &Value) -> Option<EventBody> {
    let faces = params.get("faces")?.as_array()?;
    let faces = faces
        .iter()
        .map(Value::as_u64)
        .collect::<Option<Vec<_>>>()?;
    Some(EventBody::Check {
        faces,
        stat: params.get("stat")?.as_i64()?,
        total: params.get("total")?.as_i64()?,
        target: params.get("target")?.as_i64()?,
        success: params.get("success")?.as_bool()?,
    })
}

fn text_field(params: &Value, key: &str) -> String {
    params
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn nonempty(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(text)) if !text.is_empty() => Some(text.clone()),
        _ => None,
    }
}

fn read_line(read: &mut TcpStream) -> std::io::Result<Option<String>> {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match read.read(&mut byte) {
            Ok(0) => {
                if buf.is_empty() {
                    return Ok(None);
                }
                break;
            }
            Ok(_) => {
                if byte[0] == b'\n' {
                    break;
                }
                buf.push(byte[0]);
            }
            Err(err) => return Err(err),
        }
    }
    if buf.last() == Some(&b'\r') {
        buf.pop();
    }
    Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_say_event_keeps_the_speaker_and_the_text() {
        let line = String::from_utf8(crate::protocol::write_say_event(&crate::protocol::GmSay {
            seq: 4,
            from: "player:pc_rowan".to_string(),
            address: "table".to_string(),
            place: None,
            actor: None,
            recipients: vec!["pc_rowan".to_string()],
            text: "hello".to_string(),
        }))
        .unwrap();
        let Incoming::Event(event) = classify_line(line.trim_end()).unwrap() else {
            panic!("say");
        };
        match event.body {
            EventBody::Say {
                from, seq, text, ..
            } => {
                assert_eq!(from, "player:pc_rowan");
                assert_eq!(seq, 4);
                assert_eq!(text, "hello");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_queued_event_keeps_the_actor_and_the_move() {
        let line = String::from_utf8(crate::protocol::write_queued_event(
            "id-1", "pc_rowan", "travel",
        ))
        .unwrap();
        let Incoming::Event(event) = classify_line(line.trim_end()).unwrap() else {
            panic!("queued");
        };
        assert_eq!(
            event.body,
            EventBody::Queued {
                id: "id-1".to_string(),
                actor_id: "pc_rowan".to_string(),
                move_name: "travel".to_string(),
            }
        );
    }

    #[test]
    fn a_check_event_keeps_faces_and_drops_nothing_it_was_given() {
        let line = String::from_utf8(crate::protocol::write_check_event(&crate::protocol::Roll {
            faces: vec![4, 2],
            stat: 1,
            total: 7,
            target: 0,
            success: true,
        }))
        .unwrap();
        let Incoming::Event(event) = classify_line(line.trim_end()).unwrap() else {
            panic!("check");
        };
        assert_eq!(
            event.body,
            EventBody::Check {
                faces: vec![4, 2],
                stat: 1,
                total: 7,
                target: 0,
                success: true,
            }
        );
    }

    #[test]
    fn a_refused_hello_keeps_the_token_and_the_line() {
        let line = "{\"kind\":\"refused\",\"code\":\"SESSION_FULL\"}";
        match classify_line(line).unwrap() {
            Incoming::Refused { code, raw } => {
                assert_eq!(code, "SESSION_FULL");
                assert_eq!(raw, line);
            }
            other => panic!("{other:?}"),
        }
    }
}
