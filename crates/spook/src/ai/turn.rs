use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use serde_json::json;
use spookery_kernel::canon::{self, CastKind};

use crate::ai::fail;
use crate::ai::model::{CompleteError, CompletionRequest, Model};
use crate::ai::prompt::{
    from_canon, keep, parse_actions, prepare_with, push_handled, render, HandledSay, Kept, MenuRow,
    OutgoingSay, Prepared, PromptSide, Wake,
};
use crate::ai::wire::{self, Seat, Status};
use crate::ai::HostError;

pub struct Watch {
    pub listening: AtomicBool,
    pub busy: AtomicBool,
    pub sit_seq: AtomicU64,
    pub high_water: AtomicU64,
}

impl Default for Watch {
    fn default() -> Self {
        Self {
            listening: AtomicBool::new(false),
            busy: AtomicBool::new(false),
            sit_seq: AtomicU64::new(0),
            high_water: AtomicU64::new(0),
        }
    }
}

pub fn drive<M: Model>(
    pack: &Path,
    addr: &str,
    model: &mut M,
    watch: &Watch,
) -> Result<(), HostError> {
    let (menu, kinds) = {
        let canon = canon::load_canon(pack).map_err(|err| fail(err.to_string()))?;
        from_canon(&canon)
    };
    let rev = canon::pack_rev(pack).map_err(|err| fail(err.to_string()))?;
    let mut seat = Seat::connect(addr)?;
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
        run_turn(TurnIn {
            seat: &mut seat,
            menu: &menu,
            kinds: &kinds,
            buffer: &mut buffer,
            high: &mut high,
            wake: Wake::Sit,
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
        run_turn(TurnIn {
            seat: &mut seat,
            menu: &menu,
            kinds: &kinds,
            buffer: &mut buffer,
            high: &mut high,
            wake,
            model,
            watch,
        })?;
        watch.busy.store(false, Ordering::Release);
    }
}

struct TurnIn<'a, M: Model> {
    seat: &'a mut Seat,
    menu: &'a [MenuRow],
    kinds: &'a std::collections::BTreeMap<String, CastKind>,
    buffer: &'a mut Vec<HandledSay>,
    high: &'a mut u64,
    wake: Wake,
    model: &'a mut M,
    watch: &'a Watch,
}

fn run_turn<M: Model>(turn: TurnIn<'_, M>) -> Result<(), HostError> {
    let TurnIn {
        seat,
        menu,
        kinds,
        buffer,
        high,
        wake,
        model,
        watch,
    } = turn;
    let status = call_status(seat, Some(*high))?;
    let mut seat_err = None;
    let prepared = prepare_with(kinds, &wake, &status.present, &mut |viewer| match perceive(
        seat, viewer,
    ) {
        Ok(result) => result,
        Err(err) => {
            seat_err = Some(err);
            Err("seat".to_string())
        }
    });
    if let Some(err) = seat_err {
        return Err(err);
    }
    let mut submitted = Vec::new();
    let mut view = None;
    match prepared {
        Prepared::NoModel { says } => {
            for say in says {
                if let Some(handled) = submit_say(seat, &say)? {
                    submitted.push(handled);
                }
            }
        }
        Prepared::Model(turn_view) => {
            let side = PromptSide {
                buffer: buffer.clone(),
                new_lines: player_lines(&status, *high),
                queue: status.queued.clone(),
                checks: Vec::new(),
            };
            let prompt = render(menu, &side, &turn_view, &[]);
            match model.complete(&CompletionRequest {
                prompt,
                says_only: false,
            }) {
                Err(err) => report_model(err),
                Ok(body) => {
                    if let Some(kept) = kept_actions(body, false, &turn_view.rule, menu) {
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
            view = Some((turn_view, side));
        }
    }
    let after = call_status(seat, None)?;
    let mut checks = Vec::new();
    let committed = if after.queued.is_empty() {
        false
    } else {
        let outcome = seat.call("commit_turn", json!({ "include_tick": true }))?;
        if outcome.refused {
            false
        } else {
            checks = wire::rolls_of(&outcome.body);
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
    if !checks.is_empty() {
        if let Some((turn_view, mut side)) = view {
            side.buffer = buffer.clone();
            side.checks = checks.clone();
            let prompt = render(menu, &side, &turn_view, &checks);
            match model.complete(&CompletionRequest {
                prompt,
                says_only: true,
            }) {
                Err(err) => report_model(err),
                Ok(body) => {
                    if let Some(kept) = kept_actions(body, true, &turn_view.rule, menu) {
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
    rule: &crate::ai::prompt::Rule,
    menu: &[MenuRow],
) -> Option<Kept> {
    let Some(text) = body else {
        return Some(Kept {
            says: Vec::new(),
            intent: None,
        });
    };
    if serde_json::from_str::<serde_json::Value>(&text).is_err() {
        report_model(CompleteError::EmptyParse("empty parse".to_string()));
        return None;
    }
    Some(match parse_actions(&text, says_only) {
        Some(object) => keep(object, rule, menu),
        None => Kept {
            says: Vec::new(),
            intent: None,
        },
    })
}

fn report_model(err: CompleteError) {
    eprintln!("{err}");
    let _ = std::io::Write::flush(&mut std::io::stderr());
}

fn submit_say(seat: &mut Seat, say: &OutgoingSay) -> Result<Option<HandledSay>, HostError> {
    let outcome = seat.call("say", say.arguments())?;
    if outcome.refused {
        return Ok(None);
    }
    Ok(Some(say.handled(wire::say_seq(&outcome.body))))
}

fn perceive(seat: &mut Seat, viewer: &str) -> Result<Result<serde_json::Value, String>, HostError> {
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

fn call_status(seat: &mut Seat, since: Option<u64>) -> Result<Status, HostError> {
    let arguments = match since {
        Some(since) => json!({ "since": since }),
        None => json!({}),
    };
    let outcome = seat.call("session_status", arguments)?;
    if outcome.refused {
        return Err(fail(outcome.raw));
    }
    wire::status_fields(&outcome.body).ok_or_else(|| fail(outcome.raw))
}

fn player_lines(status: &Status, high: u64) -> Vec<HandledSay> {
    status
        .events
        .iter()
        .filter(|say| say.from.starts_with("player:") && say.seq.is_some_and(|seq| seq > high))
        .cloned()
        .collect()
}
