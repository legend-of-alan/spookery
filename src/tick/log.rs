use spookery_kernel::model::{Effect, Intent, IntentArgs, Target};

pub struct Rejected {
    pub id: String,
    pub reason: String,
}

pub fn render_tick(
    tick_index: u64,
    state_rev_before: u64,
    state_rev_after: u64,
    world_time_before: u64,
    world_time_after: u64,
    presence_humans: u32,
    seed: &str,
    last_tick_at: &str,
    intents: &[Intent],
    rejected: &[Rejected],
    effects: &[Effect],
    tool_trace: &[String],
) -> String {
    let mut out = String::from("{");
    push_key_u64(&mut out, "tick_index", tick_index, true);
    push_key_u64(&mut out, "state_rev_before", state_rev_before, false);
    push_key_u64(&mut out, "state_rev_after", state_rev_after, false);
    push_key_u64(&mut out, "world_time_before", world_time_before, false);
    push_key_u64(&mut out, "world_time_after", world_time_after, false);
    push_key_u64(&mut out, "presence_humans", u64::from(presence_humans), false);
    out.push_str(",\"seed\":");
    push_string(&mut out, seed);
    out.push_str(",\"last_tick_at\":");
    push_string(&mut out, last_tick_at);
    out.push_str(",\"intents\":[");
    for (index, intent) in intents.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        push_intent(&mut out, intent);
    }
    out.push_str("],\"rejected\":[");
    for (index, item) in rejected.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str("{\"id\":");
        push_string(&mut out, &item.id);
        out.push_str(",\"reason\":");
        push_string(&mut out, &item.reason);
        out.push('}');
    }
    out.push_str("],\"effects\":[");
    for (index, effect) in effects.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        push_effect(&mut out, effect);
    }
    out.push_str("],\"tool_trace\":[");
    for (index, name) in tool_trace.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        push_string(&mut out, name);
    }
    out.push_str("]}\n");
    out
}

fn push_intent(out: &mut String, intent: &Intent) {
    out.push_str("{\"id\":");
    push_string(out, &intent.id);
    out.push_str(",\"tick_index\":");
    out.push_str(&intent.tick_index.to_string());
    out.push_str(",\"actor_id\":");
    push_string(out, &intent.actor_id);
    out.push_str(",\"actor_kind\":");
    push_string(out, intent.actor_kind.token());
    out.push_str(",\"source\":");
    push_string(out, intent.source.token());
    out.push_str(",\"move\":");
    push_string(out, &intent.move_name);
    out.push_str(",\"target\":");
    push_target(out, &intent.target);
    out.push_str(",\"args\":");
    push_args(out, &intent.args);
    out.push_str(",\"rationale_code\":");
    push_string(out, &intent.rationale_code);
    out.push_str(",\"llm_say\":null}");
}

fn push_target(out: &mut String, target: &Target) {
    match target {
        Target::Empty => out.push_str("{}"),
        Target::Place(id) => {
            out.push_str("{\"place\":");
            push_string(out, id);
            out.push('}');
        }
        Target::Actor(id) => {
            out.push_str("{\"actor\":");
            push_string(out, id);
            out.push('}');
        }
        Target::Clock(id) => {
            out.push_str("{\"clock\":");
            push_string(out, id);
            out.push('}');
        }
        Target::Rumor(id) => {
            out.push_str("{\"rumor\":");
            push_string(out, id);
            out.push('}');
        }
    }
}

fn push_args(out: &mut String, args: &IntentArgs) {
    let mut fields: Vec<(&str, ArgValue<'_>)> = Vec::new();
    if let Some(value) = &args.sku {
        fields.push(("sku", ArgValue::Str(value)));
    }
    if let Some(value) = args.delta {
        fields.push(("delta", ArgValue::Int(value)));
    }
    if let Some(value) = &args.key {
        fields.push(("key", ArgValue::Str(value)));
    }
    if let Some(value) = &args.to {
        fields.push(("to", ArgValue::Str(value)));
    } else if let Some(value) = &args.message_to {
        fields.push(("to", ArgValue::Strs(value)));
    }
    if let Some(value) = &args.message_id {
        fields.push(("message_id", ArgValue::Str(value)));
    }
    if let Some(value) = &args.due_clock {
        fields.push(("due_clock", ArgValue::Str(value)));
    }
    if let Some(value) = args.due_value {
        fields.push(("due_value", ArgValue::Int(value)));
    }
    if let Some(value) = &args.place {
        fields.push(("place", ArgValue::Str(value)));
    }
    if let Some(value) = &args.fact_ids {
        fields.push(("fact_ids", ArgValue::Strs(value)));
    }
    if let Some(value) = &args.text {
        fields.push(("text", ArgValue::Str(value)));
    }
    if let Some(value) = &args.proposal_id {
        fields.push(("proposal_id", ArgValue::Str(value)));
    }
    if let Some(value) = &args.kind {
        fields.push(("kind", ArgValue::Str(value)));
    }
    if let Some(value) = &args.target {
        fields.push(("target", ArgValue::Str(value)));
    }
    if let Some(value) = &args.summary {
        fields.push(("summary", ArgValue::Str(value)));
    }
    out.push('{');
    for (index, (key, value)) in fields.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        push_string(out, key);
        out.push(':');
        match value {
            ArgValue::Str(text) => push_string(out, text),
            ArgValue::Int(number) => out.push_str(&number.to_string()),
            ArgValue::Strs(values) => {
                out.push('[');
                for (index, item) in values.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    push_string(out, item);
                }
                out.push(']');
            }
        }
    }
    out.push('}');
}

enum ArgValue<'a> {
    Str(&'a str),
    Int(i64),
    Strs(&'a [String]),
}

fn push_effect(out: &mut String, effect: &Effect) {
    out.push_str("{\"class\":");
    push_string(out, effect.class_name());
    out.push_str(",\"as\":");
    push_string(out, &effect.as_viewer().token());
    out.push_str(",\"intent_id\":");
    push_string(out, effect.intent_id());
    out.push_str(",\"payload\":{");
    match effect {
        Effect::Clock { clock_id, delta, .. } => {
            field_str(out, "clock_id", clock_id, true);
            field_i64(out, "delta", *delta, false);
        }
        Effect::Need {
            actor_id, key, delta, ..
        }
        | Effect::Resource {
            actor_id, key, delta, ..
        } => {
            field_str(out, "actor_id", actor_id, true);
            field_str(out, "key", key, false);
            field_i64(out, "delta", *delta, false);
        }
        Effect::Stock {
            place, sku, delta, ..
        } => {
            field_str(out, "place", place, true);
            field_str(out, "sku", sku, false);
            field_i64(out, "delta", *delta, false);
        }
        Effect::Location {
            actor_id, from, to, ..
        } => {
            field_str(out, "actor_id", actor_id, true);
            field_str(out, "from", from, false);
            field_str(out, "to", to, false);
        }
        Effect::Relationship { a, b, delta, .. } => {
            field_str(out, "a", a, true);
            field_str(out, "b", b, false);
            field_i64(out, "delta", *delta, false);
        }
        Effect::Reputation {
            actor_id,
            faction,
            delta,
            ..
        } => {
            field_str(out, "actor_id", actor_id, true);
            field_str(out, "faction", faction, false);
            field_i64(out, "delta", *delta, false);
        }
        Effect::Memory {
            actor_id,
            fact_ids,
            tags,
            text,
            ..
        } => {
            field_str(out, "actor_id", actor_id, true);
            out.push_str(",\"fact_ids\":");
            push_string_list(out, fact_ids);
            out.push_str(",\"tags\":");
            push_string_list(out, tags);
            if let Some(text) = text {
                out.push_str(",\"text\":");
                push_string(out, text);
            }
        }
        Effect::Rumor {
            rumor_id,
            fact_ids,
            from,
            to,
            heat,
            ..
        } => {
            field_str(out, "rumor_id", rumor_id, true);
            out.push_str(",\"fact_ids\":");
            push_string_list(out, fact_ids);
            field_str(out, "from", from, false);
            field_str(out, "to", to, false);
            field_i64(out, "heat", *heat, false);
        }
        Effect::Flag {
            actor_id,
            key,
            value,
            ..
        } => {
            field_str(out, "actor_id", actor_id, true);
            field_str(out, "key", key, false);
            out.push_str(",\"value\":");
            out.push_str(if *value { "true" } else { "false" });
        }
        Effect::Proposal {
            proposal_id,
            kind,
            target,
            summary,
            ..
        } => {
            field_str(out, "proposal_id", proposal_id, true);
            field_str(out, "kind", kind, false);
            field_str(out, "target", target, false);
            field_str(out, "summary", summary, false);
        }
        Effect::Message {
            message_id,
            to,
            place,
            fact_ids,
            text,
            ..
        } => {
            field_str(out, "message_id", message_id, true);
            out.push_str(",\"to\":");
            push_string_list(out, to);
            if let Some(place) = place {
                field_str(out, "place", place, false);
            }
            out.push_str(",\"fact_ids\":");
            push_string_list(out, fact_ids);
            field_str(out, "text", text, false);
        }
    }
    out.push_str("}}");
}

fn push_key_u64(out: &mut String, key: &str, value: u64, first: bool) {
    if !first {
        out.push(',');
    }
    push_string(out, key);
    out.push(':');
    out.push_str(&value.to_string());
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

fn push_string_list(out: &mut String, values: &[String]) {
    out.push('[');
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        push_string(out, value);
    }
    out.push(']');
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
            other if other.is_control() => out.push_str(&format!("\\u{:04x}", other as u32)),
            other => out.push(other),
        }
    }
    out.push('"');
}
