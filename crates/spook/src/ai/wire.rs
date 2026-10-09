use serde_json::Value;
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;

use crate::fail;
use crate::prompt::Wake;
use crate::HostError;

pub struct CallOutcome {
    pub raw: String,
    pub body: Value,
    pub refused: bool,
}

enum Incoming {
    Result { id: u64, raw: String },
    Wake(Wake),
    Bad(String),
    Eof,
}

struct Shared {
    high_water: AtomicU64,
    gen: AtomicU64,
    commit_id: Mutex<Option<u64>>,
}

pub struct Seat {
    write: TcpStream,
    incoming: mpsc::Receiver<Incoming>,
    wakes: VecDeque<Wake>,
    next_id: u64,
    shared: Arc<Shared>,
}

impl Seat {
    pub fn connect(addr: &str) -> Result<Self, HostError> {
        let mut write = TcpStream::connect(addr).map_err(|err| fail(err.to_string()))?;
        write
            .set_nodelay(true)
            .map_err(|err| fail(err.to_string()))?;
        let read = write.try_clone().map_err(|err| fail(err.to_string()))?;
        let hello = format!(
            "{}{}{}",
            spookery_protocol::GM_HELLO,
            spookery_protocol::INITIALIZE_LINE,
            spookery_protocol::INITIALIZED_LINE
        );
        write
            .write_all(hello.as_bytes())
            .map_err(|err| fail(err.to_string()))?;
        write.flush().map_err(|err| fail(err.to_string()))?;
        let shared = Arc::new(Shared {
            high_water: AtomicU64::new(u64::MAX),
            gen: AtomicU64::new(0),
            commit_id: Mutex::new(None),
        });
        let (tx, incoming) = mpsc::channel();
        let shared_read = Arc::clone(&shared);
        thread::spawn(move || read_loop(read, tx, shared_read));
        let mut seat = Seat {
            write,
            incoming,
            wakes: VecDeque::new(),
            next_id: 2,
            shared,
        };
        let hello = seat.wait_id(1)?;
        if hello.refused {
            return Err(fail(hello.raw));
        }
        Ok(seat)
    }

    pub fn set_high_water(&self, seq: u64) {
        self.shared.high_water.store(seq, Ordering::Release);
    }

    pub fn call(&mut self, name: &str, arguments: Value) -> Result<CallOutcome, HostError> {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        if name == "commit_turn" {
            *self.shared.commit_id.lock().expect("commit id") = Some(id);
        }
        let line = spookery_protocol::write_tool_call(&spookery_protocol::ToolCall {
            id,
            name: name.to_string(),
            arguments,
        });
        self.write
            .write_all(line.as_bytes())
            .map_err(|err| fail(err.to_string()))?;
        self.write.flush().map_err(|err| fail(err.to_string()))?;
        self.wait_id(id)
    }

    pub fn next_wake(&mut self) -> Result<Option<Wake>, HostError> {
        if let Some(wake) = self.wakes.pop_front() {
            return Ok(Some(wake));
        }
        loop {
            match self.recv() {
                Incoming::Wake(wake) => return Ok(Some(wake)),
                Incoming::Result { raw, .. } => return Err(fail(raw)),
                Incoming::Bad(line) => return Err(fail(line)),
                Incoming::Eof => return Ok(None),
            }
        }
    }

    /// Drop say wakes already covered by `high`, and queued wakes from before a commit.
    pub fn retain_pending(&mut self, high: u64, committed: bool) {
        let gen = self.shared.gen.load(Ordering::Acquire);
        self.wakes.retain(|wake| match wake {
            Wake::Say { seq, .. } => *seq > high,
            Wake::Queued { gen: arrived } => !committed || *arrived >= gen,
            Wake::Sit => false,
        });
    }

    fn wait_id(&mut self, id: u64) -> Result<CallOutcome, HostError> {
        loop {
            match self.recv() {
                Incoming::Result { id: got, raw } if got == id => return parse_tool_line(&raw),
                Incoming::Result { raw, .. } => return Err(fail(raw)),
                Incoming::Wake(wake) => self.wakes.push_back(wake),
                Incoming::Bad(line) => return Err(fail(line)),
                Incoming::Eof => return Err(fail("seat closed".to_string())),
            }
        }
    }

    fn recv(&mut self) -> Incoming {
        match self.incoming.recv() {
            Ok(incoming) => incoming,
            Err(_) => Incoming::Eof,
        }
    }
}

impl Drop for Seat {
    fn drop(&mut self) {
        let _ = self.write.shutdown(Shutdown::Both);
    }
}

pub fn parse_tool_line(raw: &str) -> Result<CallOutcome, HostError> {
    let body = spookery_protocol::read_tool_result(raw).map_err(|_| fail(raw.to_string()))?;
    let refused = body.get("code").is_some();
    Ok(CallOutcome {
        raw: raw.to_string(),
        body,
        refused,
    })
}

fn read_loop(mut read: TcpStream, tx: mpsc::Sender<Incoming>, shared: Arc<Shared>) {
    loop {
        let line = match read_line(&mut read) {
            Ok(Some(line)) => line,
            Ok(None) | Err(_) => {
                let _ = tx.send(Incoming::Eof);
                return;
            }
        };
        match classify(&line, &shared) {
            Some(incoming) => {
                if tx.send(incoming).is_err() {
                    return;
                }
            }
            None => {}
        }
    }
}

fn classify(line: &str, shared: &Shared) -> Option<Incoming> {
    let frame = match spookery_protocol::read_frame(line) {
        Ok(frame) => frame,
        Err(_) => return Some(Incoming::Bad(line.to_string())),
    };
    let gen = shared.gen.load(Ordering::Acquire);
    match frame {
        spookery_protocol::Frame::Result { id } => {
            let commit = *shared.commit_id.lock().expect("commit id");
            if commit == Some(id) {
                shared.gen.fetch_add(1, Ordering::Release);
                *shared.commit_id.lock().expect("commit id") = None;
            }
            Some(Incoming::Result {
                id,
                raw: line.to_string(),
            })
        }
        spookery_protocol::Frame::Say {
            from,
            seq,
            address,
            place,
            actor,
            text,
        } => {
            let high = shared.high_water.load(Ordering::Acquire);
            if from.starts_with("player:") && seq > high {
                Some(Incoming::Wake(Wake::Say {
                    from,
                    seq,
                    address,
                    place,
                    actor,
                    text,
                    gen,
                }))
            } else {
                None
            }
        }
        spookery_protocol::Frame::Queued => Some(Incoming::Wake(Wake::Queued { gen })),
        spookery_protocol::Frame::Dropped => None,
    }
}

fn optional_string(value: Option<&Value>) -> Option<String> {
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

pub fn status_fields(body: &Value) -> Option<Status> {
    let present = string_list(body.get("presence")?.get("actors_present")?)?;
    let queued = body
        .get("queued")?
        .as_array()?
        .iter()
        .map(|item| {
            Some(crate::prompt::QueueItem {
                id: item.get("id")?.as_str()?.to_string(),
                actor_id: item.get("actor_id")?.as_str()?.to_string(),
                move_name: item.get("move")?.as_str()?.to_string(),
            })
        })
        .collect::<Option<Vec<_>>>()?;
    let events = match body.get("events") {
        Some(events) => events
            .as_array()?
            .iter()
            .map(say_from_event)
            .collect::<Option<Vec<_>>>()?,
        None => Vec::new(),
    };
    Some(Status {
        pack_rev: body.get("pack_rev")?.as_str()?.to_string(),
        last_seq: body.get("last_seq")?.as_u64()?,
        present,
        queued,
        events,
    })
}

pub struct Status {
    pub pack_rev: String,
    pub last_seq: u64,
    pub present: Vec<String>,
    pub queued: Vec<crate::prompt::QueueItem>,
    pub events: Vec<crate::prompt::HandledSay>,
}

fn say_from_event(value: &Value) -> Option<crate::prompt::HandledSay> {
    Some(crate::prompt::HandledSay {
        seq: value.get("seq").and_then(Value::as_u64),
        from: value.get("from")?.as_str()?.to_string(),
        address: value.get("address")?.as_str()?.to_string(),
        place: optional_string(value.get("place")),
        actor: optional_string(value.get("actor")),
        text: value.get("text")?.as_str()?.to_string(),
    })
}

fn string_list(value: &Value) -> Option<Vec<String>> {
    value
        .as_array()?
        .iter()
        .map(|item| Some(item.as_str()?.to_string()))
        .collect()
}

pub fn rolls_of(body: &Value) -> Vec<crate::prompt::Roll> {
    let Some(checks) = body.get("checks").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in checks {
        let Some(faces) = item.get("faces").and_then(Value::as_array) else {
            continue;
        };
        let faces = faces.iter().filter_map(Value::as_u64).collect::<Vec<_>>();
        let Some(stat) = item.get("stat").and_then(Value::as_i64) else {
            continue;
        };
        let Some(total) = item.get("total").and_then(Value::as_i64) else {
            continue;
        };
        let Some(target) = item.get("target").and_then(Value::as_i64) else {
            continue;
        };
        let Some(success) = item.get("success").and_then(Value::as_bool) else {
            continue;
        };
        out.push(crate::prompt::Roll {
            faces,
            stat,
            total,
            target,
            success,
        });
    }
    out
}

pub fn say_seq(body: &Value) -> Option<u64> {
    body.get("seq").and_then(Value::as_u64)
}
