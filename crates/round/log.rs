use spookery_kernel::effect::Effect;
use spookery_kernel::intent::{Intent, IntentArgs, Target};

pub struct Rejected {
    pub id: String,
    pub reason: String,
}

pub struct RoundLine<'a> {
    pub tick_index: u64,
    pub rules_rev: u64,
    pub state_rev_before: u64,
    pub state_rev_after: u64,
    pub world_time_before: u64,
    pub world_time_after: u64,
    pub presence_humans: u32,
    pub seed: &'a str,
    pub last_tick_at: &'a str,
    pub intents: &'a [Intent],
    pub rejected: &'a [Rejected],
    pub effects: &'a [Effect],
    pub tool_trace: &'a [String],
}

pub fn render_round_line(line: &RoundLine<'_>) -> String {
    let mut out = String::from("{");
    push_key_u64(&mut out, "tick_index", line.tick_index, true);
    push_key_u64(&mut out, "rules_rev", line.rules_rev, false);
    push_key_u64(&mut out, "state_rev_before", line.state_rev_before, false);
    push_key_u64(&mut out, "state_rev_after", line.state_rev_after, false);
    push_key_u64(&mut out, "world_time_before", line.world_time_before, false);
    push_key_u64(&mut out, "world_time_after", line.world_time_after, false);
    push_key_u64(
        &mut out,
        "presence_humans",
        u64::from(line.presence_humans),
        false,
    );
    out.push_str(",\"seed\":");
    push_string(&mut out, line.seed);
    out.push_str(",\"last_tick_at\":");
    push_string(&mut out, line.last_tick_at);
    out.push_str(",\"intents\":[");
    for (index, intent) in line.intents.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        push_intent(&mut out, intent);
    }
    out.push_str("],\"rejected\":[");
    for (index, item) in line.rejected.iter().enumerate() {
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
    for (index, effect) in line.effects.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        push_effect(&mut out, effect);
    }
    out.push_str("],\"tool_trace\":[");
    for (index, name) in line.tool_trace.iter().enumerate() {
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
    out.push_str(&spookery_kernel::store::codec::emit_effect(effect));
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
    use spookery_kernel::intent::ViewerId;

    use super::{render_round_line, RoundLine};

    #[test]
    fn round_line_embeds_the_flat_effect() {
        let effect = Effect {
            class: EffectClass::Item {
                actor_id: None,
                place: Some("docks_shop".to_string()),
                id: "lamp_oil".to_string(),
                delta: -1,
                max: None,
            },
            as_viewer: ViewerId::Npc("mira".to_string()),
            intent_id: "i".to_string(),
        };
        let line = render_round_line(&RoundLine {
            tick_index: 1,
            rules_rev: spookery_kernel::store::RULES_REV,
            state_rev_before: 0,
            state_rev_after: 1,
            world_time_before: 0,
            world_time_after: 60,
            presence_humans: 0,
            seed: "seed",
            last_tick_at: "2026-01-01T00:00:00Z",
            intents: &[],
            rejected: &[],
            effects: std::slice::from_ref(&effect),
            tool_trace: &[],
        });
        let encoded = spookery_kernel::store::codec::emit_effect(&effect);
        assert!(line.contains(&encoded), "{line}");
        assert!(!line.contains("\"payload\""));
    }
}
