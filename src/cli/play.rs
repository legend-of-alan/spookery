use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use spookery_kernel::digest::{idle_digest, DigestViewer};
use spookery_kernel::model::{ActorKind, ActorLive, Intent, IntentArgs, Now, Source, Target};
use spookery_kernel::perceive::{perceive, ClockView, KnownFact, Perceive, StockView};
use spookery_kernel::store::{open_shard, Shard};
use spookery_kernel::Code;

pub fn play(args: &[String]) -> Result<(), Code> {
    if args.is_empty() {
        return Err(Code::NotInSpec);
    }
    let mut shard = open_shard(Path::new(&args[0]))?;
    let default_actor = default_actor(&shard, args)?;
    for line in io::stdin().lines() {
        let line = line.map_err(|_| Code::CommitConflict)?;
        let intent = match parse_line(&line, &default_actor, shard.now()) {
            Ok(intent) => intent,
            Err(code) => {
                eprintln!("{code}");
                continue;
            }
        };
        let actor_id = intent.actor_id.clone();
        shard.queue_intent(intent);
        spookery_tick::commit_turn(&mut shard)?;
        let digest = idle_digest(&shard, DigestViewer::Player(actor_id.clone()))?;
        let view = perceive(&shard, &format!("player:{actor_id}"))?;
        print!("{digest}{}", render_perceive(&view));
    }
    Ok(())
}

fn default_actor(shard: &Shard, args: &[String]) -> Result<String, Code> {
    let id = if args.len() == 1 {
        shard.canon().start_pc.clone()
    } else if args.len() == 3 && args[1] == "--as" {
        let Some(id) = args[2].strip_prefix("player:") else {
            return Err(Code::NotInSpec);
        };
        if id.is_empty() {
            return Err(Code::Schema);
        }
        id.to_string()
    } else {
        return Err(Code::NotInSpec);
    };
    match shard.now().actors.get(&id) {
        Some(ActorLive::Pc { .. }) => Ok(id),
        _ => Err(Code::Schema),
    }
}

fn parse_line(line: &str, default_actor: &str, now: &Now) -> Result<Intent, Code> {
    let tokens: Vec<&str> = if line.is_empty() {
        Vec::new()
    } else {
        line.split(' ').collect()
    };
    if tokens.is_empty() || tokens[0].is_empty() {
        return Err(Code::Schema);
    }
    let (actor_id, rest) = if let Some(id) = tokens[0].strip_prefix("player:") {
        if id.is_empty() || !now.actors.get(id).is_some_and(ActorLive::is_pc) {
            return Err(Code::Schema);
        }
        (id.to_string(), &tokens[1..])
    } else {
        (default_actor.to_string(), tokens.as_slice())
    };
    if rest.is_empty() || rest[0].is_empty() {
        return Err(Code::Schema);
    }
    let (move_name, target, args) = parse_verb(rest[0], &rest[1..], &actor_id, now)?;
    Ok(Intent {
        id: uuid_v4()?,
        tick_index: now.tick_index.saturating_add(1),
        actor_id,
        actor_kind: ActorKind::Player,
        source: Source::Player,
        move_name,
        target,
        args,
        rationale_code: String::new(),
    })
}

fn parse_verb(
    verb: &str,
    args: &[&str],
    actor_id: &str,
    now: &Now,
) -> Result<(String, Target, IntentArgs), Code> {
    let move_name = verb.to_string();
    match verb {
        "wait" | "work" => {
            if !args.is_empty() {
                return Err(Code::Schema);
            }
            Ok((move_name, Target::Empty, IntentArgs::default()))
        }
        "travel" => Ok((
            move_name,
            Target::Place(one(args)?.to_string()),
            IntentArgs::default(),
        )),
        "restock" => {
            if args.len() != 2 {
                return Err(Code::Schema);
            }
            Ok((
                move_name,
                Target::Empty,
                IntentArgs {
                    sku: Some(token(args[0])?.to_string()),
                    delta: Some(integer(args[1])?),
                    ..IntentArgs::default()
                },
            ))
        }
        "rest" => Ok((
            move_name,
            Target::Empty,
            IntentArgs {
                delta: Some(integer(one(args)?)?),
                ..IntentArgs::default()
            },
        )),
        "pay_upkeep" => {
            if args.len() != 2 {
                return Err(Code::Schema);
            }
            Ok((
                move_name,
                Target::Empty,
                IntentArgs {
                    key: Some(token(args[0])?.to_string()),
                    delta: Some(integer(args[1])?),
                    ..IntentArgs::default()
                },
            ))
        }
        "drift_relationship" => {
            let other = one(args)?;
            Ok((
                move_name,
                Target::Actor(other.to_string()),
                IntentArgs {
                    delta: drift_delta(now, actor_id, other),
                    ..IntentArgs::default()
                },
            ))
        }
        "spread_rumor" => {
            if args.len() != 2 {
                return Err(Code::Schema);
            }
            Ok((
                move_name,
                Target::Rumor(token(args[0])?.to_string()),
                IntentArgs {
                    to: Some(token(args[1])?.to_string()),
                    ..IntentArgs::default()
                },
            ))
        }
        "leave_note" => {
            if args.len() < 6 {
                return Err(Code::Schema);
            }
            let text = args[5..].join(" ");
            if text.is_empty() {
                return Err(Code::Schema);
            }
            Ok((
                move_name,
                Target::Empty,
                IntentArgs {
                    message_id: Some(token(args[0])?.to_string()),
                    due_clock: Some(token(args[1])?.to_string()),
                    due_value: Some(integer(args[2])?),
                    message_to: Some(vec![token(args[3])?.to_string()]),
                    place: Some(token(args[4])?.to_string()),
                    text: Some(text),
                    ..IntentArgs::default()
                },
            ))
        }
        "file_proposal" => {
            if args.len() < 4 {
                return Err(Code::Schema);
            }
            let summary = args[3..].join(" ");
            if summary.is_empty() {
                return Err(Code::Schema);
            }
            Ok((
                move_name,
                Target::Empty,
                IntentArgs {
                    proposal_id: Some(token(args[0])?.to_string()),
                    kind: Some(token(args[1])?.to_string()),
                    target: Some(token(args[2])?.to_string()),
                    summary: Some(summary),
                    ..IntentArgs::default()
                },
            ))
        }
        "advance_clock" => {
            let (clock, delta) = match args {
                [clock] => (token(clock)?, 1),
                [clock, delta] => (token(clock)?, integer(delta)?),
                _ => return Err(Code::Schema),
            };
            Ok((
                move_name,
                Target::Clock(clock.to_string()),
                IntentArgs {
                    delta: Some(delta),
                    ..IntentArgs::default()
                },
            ))
        }
        "talk" | "attack" => Ok((
            move_name,
            Target::Actor(one(args)?.to_string()),
            IntentArgs::default(),
        )),
        "attempt" => {
            let _check = one(args)?;
            Ok((move_name, Target::Empty, IntentArgs::default()))
        }
        "give" | "take" => {
            if args.len() != 2 {
                return Err(Code::Schema);
            }
            let _item = token(args[0])?;
            Ok((
                move_name,
                Target::Actor(token(args[1])?.to_string()),
                IntentArgs::default(),
            ))
        }
        _ => Err(Code::UnknownMove),
    }
}

fn drift_delta(now: &Now, actor_id: &str, other: &str) -> Option<i64> {
    let relation = now
        .actors
        .get(actor_id)?
        .relationships()?
        .get(other)?;
    Some((relation.target - relation.stance).clamp(-1, 1))
}

fn one<'a>(args: &'a [&'a str]) -> Result<&'a str, Code> {
    if args.len() == 1 {
        token(args[0])
    } else {
        Err(Code::Schema)
    }
}

fn token(token: &str) -> Result<&str, Code> {
    if token.is_empty() {
        Err(Code::Schema)
    } else {
        Ok(token)
    }
}

fn integer(token: &str) -> Result<i64, Code> {
    let digits = token.strip_prefix('-').unwrap_or(token);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(Code::Schema);
    }
    token.parse().map_err(|_| Code::Schema)
}

fn uuid_v4() -> Result<String, Code> {
    let mut bytes = [0u8; 16];
    let mut file = File::open("/dev/urandom").map_err(|_| Code::CommitConflict)?;
    file.read_exact(&mut bytes).map_err(|_| Code::CommitConflict)?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Ok(format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15],
    ))
}

fn render_perceive(view: &Perceive) -> String {
    let mut out = String::from("{");
    field_str(&mut out, "viewer", &view.viewer, true);
    field_str(&mut out, "place", &view.place, false);
    out.push_str(",\"visible_actors\":");
    push_strings(&mut out, &view.visible_actors);
    out.push_str(",\"known_facts\":[");
    for (index, fact) in view.known_facts.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        push_fact(&mut out, fact);
    }
    out.push_str("],\"messages\":[");
    for (index, message) in view.messages.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        push_message(&mut out, message);
    }
    out.push_str("],\"unknown_mentioned\":");
    push_strings(&mut out, &view.unknown_mentioned);
    out.push_str(",\"clocks\":[");
    for (index, clock) in view.clocks.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        push_clock(&mut out, clock);
    }
    out.push_str("],\"stocks\":[");
    for (index, stock) in view.stocks.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        push_stock(&mut out, stock);
    }
    out.push_str("],\"schedule_hint\":null}\n");
    out
}

fn push_fact(out: &mut String, fact: &KnownFact) {
    out.push('{');
    field_str(out, "id", &fact.id, true);
    field_str(out, "text", &fact.text, false);
    field_str(out, "body", &fact.body, false);
    out.push('}');
}

fn push_message(out: &mut String, message: &spookery_kernel::model::Message) {
    out.push('{');
    field_str(out, "id", &message.id, true);
    out.push_str(",\"to\":");
    push_strings(out, &message.to);
    if let Some(place) = &message.place {
        field_str(out, "place", place, false);
    }
    out.push_str(",\"fact_ids\":");
    push_strings(out, &message.fact_ids);
    field_str(out, "text", &message.text, false);
    out.push('}');
}

fn push_clock(out: &mut String, clock: &ClockView) {
    out.push('{');
    field_str(out, "id", &clock.id, true);
    field_i64(out, "value", clock.value, false);
    field_i64(out, "max", clock.max, false);
    field_bool(out, "paused", clock.paused, false);
    out.push('}');
}

fn push_stock(out: &mut String, stock: &StockView) {
    out.push('{');
    field_str(out, "sku", &stock.sku, true);
    field_i64(out, "qty", stock.qty, false);
    out.push('}');
}

fn push_strings(out: &mut String, values: &[String]) {
    out.push('[');
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        push_string(out, value);
    }
    out.push(']');
}

fn field_str(out: &mut String, key: &str, value: &str, first: bool) {
    if !first {
        out.push(',');
    }
    push_string(out, key);
    out.push(':');
    push_string(out, value);
}

fn field_i64(out: &mut String, key: &str, value: i64, first: bool) {
    if !first {
        out.push(',');
    }
    push_string(out, key);
    out.push(':');
    out.push_str(&value.to_string());
}

fn field_bool(out: &mut String, key: &str, value: bool, first: bool) {
    if !first {
        out.push(',');
    }
    push_string(out, key);
    out.push(':');
    out.push_str(if value { "true" } else { "false" });
}

fn push_string(out: &mut String, value: &str) {
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other if other.is_control() => out.push_str(&format!("\\u{:04x}", u32::from(other))),
            other => out.push(other),
        }
    }
    out.push('"');
}
