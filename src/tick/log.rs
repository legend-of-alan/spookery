use spookery_kernel::effect::Effect;
use spookery_kernel::model::{Intent, IntentArgs, Target};

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
    push_key_u64(
        &mut out,
        "presence_humans",
        u64::from(presence_humans),
        false,
    );
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
    out.push('}');
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
    out.push('{');
    if let Some(values) = &args.message_to {
        push_string(out, "to");
        out.push_str(":[");
        for (index, item) in values.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            push_string(out, item);
        }
        out.push(']');
    }
    out.push('}');
}

fn push_effect(out: &mut String, effect: &Effect) {
    out.push_str(&spookery_kernel::codec::emit_effect(effect));
}

fn push_key_u64(out: &mut String, key: &str, value: u64, first: bool) {
    if !first {
        out.push(',');
    }
    push_string(out, key);
    out.push(':');
    out.push_str(&value.to_string());
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

#[cfg(test)]
mod tests {
    use spookery_kernel::effect::{Effect, EffectClass};
    use spookery_kernel::model::ViewerId;

    use super::render_tick;

    #[test]
    fn tick_line_embeds_the_flat_effect() {
        let effect = Effect {
            class: EffectClass::Stock {
                place: "docks_shop".to_string(),
                sku: "lamp_oil".to_string(),
                delta: -1,
            },
            as_viewer: ViewerId::Npc("mira".to_string()),
            intent_id: "i".to_string(),
        };
        let line = render_tick(
            1,
            0,
            1,
            0,
            60,
            0,
            "seed",
            "2026-01-01T00:00:00Z",
            &[],
            &[],
            &[effect.clone()],
            &[],
        );
        let encoded = spookery_kernel::codec::emit_effect(&effect);
        assert!(line.contains(&encoded), "{line}");
        assert!(!line.contains("\"payload\""));
    }
}
