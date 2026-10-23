//! Sit the GM seat and turn model replies into tool calls.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use serde_json::{json, Value};
use spookery_engine::canon::{self, CastKind};

use crate::ai::fail;
use crate::ai::llm::{CompleteError, CompletionRequest, Model};
use crate::ai::prompt::{
    from_canon, keep_actions, parse_actions, prepare_with, push_handled, render_prompt, HandledSay,
    KeptActions, MenuRow, OutgoingSay, PreparedTurn, PromptContext, Prose, SceneScope, Wake,
};
use crate::ai::Error;
use crate::gm_connection::{self, EventBody, Incoming, ToolCallResult};
use crate::protocol::{CommitBody, GmSay, SessionStatus};

pub struct TurnWatch {
    pub listening: AtomicBool,
    pub busy: AtomicBool,
    pub sit_seq: AtomicU64,
    pub high_water: AtomicU64,
}

impl Default for TurnWatch {
    fn default() -> Self {
        Self {
            listening: AtomicBool::new(false),
            busy: AtomicBool::new(false),
            sit_seq: AtomicU64::new(0),
            high_water: AtomicU64::new(0),
        }
    }
}

pub fn run_gm<M: Model>(
    pack: &Path,
    addr: &str,
    door: Option<&str>,
    model: &mut M,
    watch: &TurnWatch,
) -> Result<(), Error> {
    let (menu, kinds, prose) = {
        let canon = canon::load_canon(pack)
            .map_err(|err| fail(spookery_engine::reason_of(err).to_string()))?;
        from_canon(&canon)
    };
    let rev = canon::pack_rev(pack).map_err(|err| fail(err.to_string()))?;
    let mut seat = GmSeat::connect(addr, door)?;
    let status = call_status(&mut seat, None)?;
    if status.pack_rev != rev {
        return Err(fail("PACK_REV_MISMATCH".to_string()));
    }
    eprintln!("listening");
    let _ = std::io::Write::flush(&mut std::io::stderr());
    let mut high = status.last_seq;
    seat.set_high_water(high);
    watch.sit_seq.store(high, Ordering::Release);
    watch.high_water.store(high, Ordering::Release);
    watch.listening.store(true, Ordering::Release);
    let mut buffer = Vec::new();
    if !status.queued.is_empty() {
        watch.busy.store(true, Ordering::Release);
        run_turn(TurnContext {
            seat: &mut seat,
            menu: &menu,
            kinds: &kinds,
            prose: &prose,
            buffer: &mut buffer,
            high: &mut high,
            wake: Wake::Queued,
            model,
            watch,
        })?;
        watch.busy.store(false, Ordering::Release);
    }
    loop {
        let Some(wake) = seat.next_wake()? else {
            return Ok(());
        };
        watch.busy.store(true, Ordering::Release);
        run_turn(TurnContext {
            seat: &mut seat,
            menu: &menu,
            kinds: &kinds,
            prose: &prose,
            buffer: &mut buffer,
            high: &mut high,
            wake,
            model,
            watch,
        })?;
        watch.busy.store(false, Ordering::Release);
    }
}

struct TurnContext<'a, M: Model> {
    seat: &'a mut GmSeat,
    menu: &'a [MenuRow],
    kinds: &'a std::collections::BTreeMap<String, CastKind>,
    prose: &'a Prose,
    buffer: &'a mut Vec<HandledSay>,
    high: &'a mut u64,
    wake: Wake,
    model: &'a mut M,
    watch: &'a TurnWatch,
}

fn run_turn<M: Model>(turn: TurnContext<'_, M>) -> Result<(), Error> {
    let TurnContext {
        seat,
        menu,
        kinds,
        prose,
        buffer,
        high,
        wake,
        model,
        watch,
    } = turn;
    let status = call_status(seat, Some(*high))?;
    let mut seat_err = None;
    let prepared = prepare_with(
        kinds,
        prose,
        &wake,
        &status.presence.actors_present,
        &mut |viewer| match perceive(seat, viewer) {
            Ok(result) => result,
            Err(err) => {
                seat_err = Some(err);
                Err("seat".to_string())
            }
        },
    );
    if let Some(err) = seat_err {
        return Err(err);
    }
    let mut submitted = Vec::new();
    let mut view = None;
    match prepared {
        PreparedTurn::NoModel { says } => {
            for say in says {
                if let Some(handled) = submit_say(seat, &say)? {
                    submitted.push(handled);
                }
            }
        }
        PreparedTurn::Model(turn_view) => {
            let context = PromptContext {
                buffer: buffer.clone(),
                new_lines: player_lines(&status, *high),
                queue: status.queued.clone(),
                checks: Vec::new(),
            };
            let prompt = render_prompt(menu, &context, &turn_view, &[]);
            match model.complete(&CompletionRequest {
                prompt,
                says_only: false,
            }) {
                Err(err) => report_model(err),
                Ok(body) => {
                    if let Some(kept) = kept_actions(body, false, &turn_view.scope, menu) {
                        for say in kept.says {
                            if let Some(handled) = submit_say(seat, &say)? {
                                submitted.push(handled);
                            }
                        }
                        if let Some(intent) = kept.intent {
                            let outcome = seat.call("intent", intent.arguments())?;
                            let _ = outcome.refused;
                        }
                    }
                }
            }
            view = Some((turn_view, context));
        }
    }
    let after = call_status(seat, None)?;
    let mut checks = Vec::new();
    let committed = if after.queued.is_empty() {
        false
    } else {
        let outcome = seat.call("commit_round", json!({ "include_tick": true }))?;
        if outcome.refused {
            false
        } else {
            let commit: CommitBody = serde_json::from_value(outcome.body.clone())
                .map_err(|_| fail(outcome.raw.clone()))?;
            checks = commit.checks;
            true
        }
    };
    let window = player_lines(&status, *high);
    if let Some(max) = window.iter().filter_map(|say| say.seq).max() {
        *high = max;
        seat.set_high_water(*high);
        watch.high_water.store(*high, Ordering::Release);
    }
    for say in window {
        push_handled(buffer, say);
    }
    for say in submitted {
        push_handled(buffer, say);
    }
    seat.retain_pending(*high, committed);
    // Queued events and the commit result travel on different streams, so ask the host.
    if committed && !call_status(seat, None)?.queued.is_empty() {
        seat.wakes.push_back(Wake::Queued);
    }
    if !checks.is_empty() {
        if let Some((turn_view, mut context)) = view {
            context.buffer = buffer.clone();
            context.checks = checks.clone();
            let prompt = render_prompt(menu, &context, &turn_view, &checks);
            match model.complete(&CompletionRequest {
                prompt,
                says_only: true,
            }) {
                Err(err) => report_model(err),
                Ok(body) => {
                    if let Some(kept) = kept_actions(body, true, &turn_view.scope, menu) {
                        for say in kept.says {
                            if let Some(handled) = submit_say(seat, &say)? {
                                push_handled(buffer, handled);
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

fn kept_actions(
    body: Option<String>,
    says_only: bool,
    scope: &SceneScope,
    menu: &[MenuRow],
) -> Option<KeptActions> {
    let Some(text) = body else {
        return Some(KeptActions {
            says: Vec::new(),
            intent: None,
        });
    };
    if serde_json::from_str::<serde_json::Value>(&text).is_err() {
        report_model(CompleteError::EmptyParse("empty parse".to_string()));
        return None;
    }
    Some(match parse_actions(&text, says_only) {
        Some(actions) => keep_actions(actions, scope, menu),
        None => KeptActions {
            says: Vec::new(),
            intent: None,
        },
    })
}

fn report_model(err: CompleteError) {
    eprintln!("{err}");
    let _ = std::io::Write::flush(&mut std::io::stderr());
}

fn submit_say(seat: &mut GmSeat, say: &OutgoingSay) -> Result<Option<HandledSay>, Error> {
    let outcome = seat.call("say", say.arguments())?;
    if outcome.refused {
        return Ok(None);
    }
    let spoken: GmSay = serde_json::from_value(outcome.body).map_err(|_| fail(outcome.raw))?;
    Ok(Some(HandledSay::from_outgoing(say, Some(spoken.seq))))
}

fn perceive(seat: &mut GmSeat, viewer: &str) -> Result<Result<serde_json::Value, String>, Error> {
    let outcome = seat.call("perceive", json!({ "as": viewer }))?;
    if outcome.refused {
        let code = outcome
            .body
            .get("code")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("code")
            .to_string();
        return Ok(Err(code));
    }
    Ok(Ok(outcome.body))
}

fn call_status(seat: &mut GmSeat, since: Option<u64>) -> Result<SessionStatus, Error> {
    let arguments = match since {
        Some(since) => json!({ "since": since }),
        None => json!({}),
    };
    let outcome = seat.call("session_status", arguments)?;
    if outcome.refused {
        return Err(fail(outcome.raw));
    }
    serde_json::from_value(outcome.body).map_err(|_| fail(outcome.raw))
}

fn player_lines(status: &SessionStatus, high: u64) -> Vec<HandledSay> {
    status
        .events
        .iter()
        .flatten()
        .filter(|say| say.from.starts_with("player:") && say.seq > high)
        .cloned()
        .map(HandledSay::from)
        .collect()
}

struct GmSeat {
    client: gm_connection::GmConnection,
    wakes: VecDeque<Wake>,
    /// Say events at or below this seq are not a wake. Starts closed.
    high_water: u64,
}

impl GmSeat {
    fn connect(addr: &str, door: Option<&str>) -> Result<Self, Error> {
        let client = match door {
            Some(door) => gm_connection::GmConnection::connect_door(door),
            None => gm_connection::GmConnection::connect(addr),
        }
        .map_err(from_connection_err)?;
        Ok(GmSeat {
            client,
            wakes: VecDeque::new(),
            high_water: u64::MAX,
        })
    }

    fn set_high_water(&mut self, seq: u64) {
        self.high_water = seq;
    }

    fn call(&mut self, name: &str, arguments: Value) -> Result<ToolCallResult, Error> {
        let outcome = self
            .client
            .call(name, arguments)
            .map_err(from_connection_err)?;
        let events = self.client.take_events();
        self.absorb(events);
        Ok(outcome)
    }

    fn next_wake(&mut self) -> Result<Option<Wake>, Error> {
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

    /// Drop say wakes already covered by `high`, and queued wakes once a commit ran.
    fn retain_pending(&mut self, high: u64, committed: bool) {
        self.wakes.retain(|wake| match wake {
            Wake::Say { seq, .. } => *seq > high,
            Wake::Queued => !committed,
        });
    }

    fn absorb(&mut self, events: Vec<gm_connection::Event>) {
        for event in events {
            if let Some(wake) = self.classify(event.body) {
                self.wakes.push_back(wake);
            }
        }
    }

    fn classify(&self, body: EventBody) -> Option<Wake> {
        match body {
            EventBody::Say(say) => {
                if say.from.starts_with("player:") && say.seq > self.high_water {
                    Some(Wake::Say {
                        from: say.from,
                        seq: say.seq,
                        address: say.address,
                        place: say.place,
                        actor: say.actor,
                        text: say.text,
                    })
                } else {
                    None
                }
            }
            EventBody::Queued(_) => Some(Wake::Queued),
            EventBody::Check(_) => None,
        }
    }
}

fn from_connection_err(err: gm_connection::Error) -> Error {
    match err {
        gm_connection::Error::Io(err) => fail(err.to_string()),
        gm_connection::Error::Closed => fail("seat closed".to_string()),
        gm_connection::Error::Bad(line) => fail(line),
        gm_connection::Error::Refused { raw, .. } => fail(raw),
    }
}
