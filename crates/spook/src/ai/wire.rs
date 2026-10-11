use serde_json::Value;
use std::collections::VecDeque;

use crate::ai::fail;
use crate::ai::prompt::Wake;
use crate::ai::HostError;
use crate::gm_client::{self, EventBody, Incoming};

pub struct CallOutcome {
    pub raw: String,
    pub body: Value,
    pub refused: bool,
}

pub struct Seat {
    client: gm_client::Client,
    wakes: VecDeque<Wake>,
    /// Say events at or below this seq are not a wake. Starts closed.
    high_water: u64,
    /// Advances when a `commit_turn` result arrives, before later events.
    gen: u64,
}

impl Seat {
    pub fn connect(addr: &str) -> Result<Self, HostError> {
        let client = gm_client::Client::connect(addr).map_err(host_err)?;
        Ok(Seat {
            client,
            wakes: VecDeque::new(),
            high_water: u64::MAX,
            gen: 0,
        })
    }

    pub fn set_high_water(&mut self, seq: u64) {
        self.high_water = seq;
    }

    pub fn call(&mut self, name: &str, arguments: Value) -> Result<CallOutcome, HostError> {
        let commit = name == "commit_turn";
        let outcome = self.client.call(name, arguments).map_err(host_err)?;
        // Lines already on the socket are folded in before the commit generation
        // moves, which is the order the seat used to stamp them.
        let events = self.client.take_events();
        self.absorb(events);
        if commit {
            self.gen = self.gen.wrapping_add(1);
        }
        Ok(CallOutcome {
            raw: outcome.raw,
            body: outcome.body,
            refused: outcome.refused,
        })
    }

    pub fn next_wake(&mut self) -> Result<Option<Wake>, HostError> {
        if let Some(wake) = self.wakes.pop_front() {
            return Ok(Some(wake));
        }
        loop {
            match self.client.next_incoming() {
                Incoming::Event(event) => {
                    if let Some(wake) = self.classify(event.body) {
                        return Ok(Some(wake));
                    }
                }
                Incoming::Result { raw, .. }
                | Incoming::Bad(raw)
                | Incoming::Refused { raw, .. } => {
                    return Err(fail(raw));
                }
                Incoming::Eof => return Ok(None),
            }
        }
    }

    /// Drop say wakes already covered by `high`, and queued wakes from before a commit.
    pub fn retain_pending(&mut self, high: u64, committed: bool) {
        let gen = self.gen;
        self.wakes.retain(|wake| match wake {
            Wake::Say { seq, .. } => *seq > high,
            Wake::Queued { gen: arrived } => !committed || *arrived >= gen,
            Wake::Sit => false,
        });
    }

    fn absorb(&mut self, events: Vec<gm_client::Event>) {
        for event in events {
            if let Some(wake) = self.classify(event.body) {
                self.wakes.push_back(wake);
            }
        }
    }

    fn classify(&self, body: EventBody) -> Option<Wake> {
        let gen = self.gen;
        match body {
            EventBody::Say {
                from,
                seq,
                address,
                place,
                actor,
                text,
            } => {
                if from.starts_with("player:") && seq > self.high_water {
                    Some(Wake::Say {
                        from,
                        seq,
                        address,
                        place,
                        actor,
                        text,
                        gen,
                    })
                } else {
                    None
                }
            }
            EventBody::Queued { .. } => Some(Wake::Queued { gen }),
            EventBody::Check { .. } => None,
        }
    }
}

fn host_err(err: gm_client::Error) -> HostError {
    match err {
        gm_client::Error::Io(err) => fail(err.to_string()),
        gm_client::Error::Closed => fail("seat closed".to_string()),
        gm_client::Error::Bad(line) => fail(line),
        gm_client::Error::Refused { raw, .. } => fail(raw),
    }
}

pub fn parse_tool_line(raw: &str) -> Result<CallOutcome, HostError> {
    let body = crate::protocol::read_tool_result(raw).map_err(|_| fail(raw.to_string()))?;
    let refused = body.get("code").is_some();
    Ok(CallOutcome {
        raw: raw.to_string(),
        body,
        refused,
    })
}

pub fn status_fields(body: &Value) -> Option<Status> {
    let present = string_list(body.get("presence")?.get("actors_present")?)?;
    let queued = body
        .get("queued")?
        .as_array()?
        .iter()
        .map(|item| {
            Some(crate::ai::prompt::QueueItem {
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
    pub queued: Vec<crate::ai::prompt::QueueItem>,
    pub events: Vec<crate::ai::prompt::HandledSay>,
}

fn say_from_event(value: &Value) -> Option<crate::ai::prompt::HandledSay> {
    Some(crate::ai::prompt::HandledSay {
        seq: value.get("seq").and_then(Value::as_u64),
        from: value.get("from")?.as_str()?.to_string(),
        address: value.get("address")?.as_str()?.to_string(),
        place: optional_string(value.get("place")),
        actor: optional_string(value.get("actor")),
        text: value.get("text")?.as_str()?.to_string(),
    })
}

fn optional_string(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(text)) if !text.is_empty() => Some(text.clone()),
        _ => None,
    }
}

fn string_list(value: &Value) -> Option<Vec<String>> {
    value
        .as_array()?
        .iter()
        .map(|item| Some(item.as_str()?.to_string()))
        .collect()
}

pub fn rolls_of(body: &Value) -> Vec<crate::ai::prompt::Roll> {
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
        let Some(outcome) = item.get("outcome").and_then(Value::as_str) else {
            continue;
        };
        if !matches!(outcome, "success" | "partial" | "miss") {
            continue;
        }
        out.push(crate::ai::prompt::Roll {
            faces,
            stat,
            total,
            target,
            outcome: outcome.to_string(),
        });
    }
    out
}

pub fn say_seq(body: &Value) -> Option<u64> {
    body.get("seq").and_then(Value::as_u64)
}
