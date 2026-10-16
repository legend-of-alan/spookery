use spookery_kernel::effect::Effect;
use spookery_kernel::intent::Intent;
use spookery_kernel::journal::{self, Arrival, Rejected, TickRecord};

pub struct RoundLine<'a> {
    pub tick_index: u64,
    pub rules_rev: u64,
    pub state_rev_before: u64,
    pub state_rev_after: u64,
    pub world_time_before: u64,
    pub world_time_after: u64,
    pub presence_humans: u32,
    pub actors_present: &'a [String],
    pub seed: &'a str,
    pub last_tick_at: &'a str,
    pub intents: &'a [Intent],
    pub rejected: &'a [Rejected],
    pub effects: &'a [Effect],
    pub arrivals: &'a [Arrival],
    pub fast_forward: bool,
    pub tool_trace: &'a [String],
}

pub fn render_round_line(line: &RoundLine<'_>) -> String {
    let record = TickRecord {
        tick_index: line.tick_index,
        rules_rev: line.rules_rev,
        state_rev_before: line.state_rev_before,
        state_rev_after: line.state_rev_after,
        world_time_before: line.world_time_before,
        world_time_after: line.world_time_after,
        presence_humans: line.presence_humans,
        actors_present: line.actors_present.to_vec(),
        seed: line.seed.to_string(),
        last_tick_at: line.last_tick_at.to_string(),
        intents: line.intents.to_vec(),
        rejected: line.rejected.to_vec(),
        effects: line.effects.to_vec(),
        arrivals: line.arrivals.to_vec(),
        fast_forward: line.fast_forward,
        tool_trace: line.tool_trace.to_vec(),
        now_hash: String::new(),
    };
    journal::emit_entry(&journal::JournalEntry::Tick(record))
}

#[cfg(test)]
mod tests {
    use spookery_kernel::effect::{Effect, EffectKind};
    use spookery_kernel::intent::ViewerId;

    use super::{render_round_line, RoundLine};

    #[test]
    fn round_line_embeds_the_flat_effect() {
        let effect = Effect {
            class: EffectKind::Item {
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
            actors_present: &[],
            seed: "seed",
            last_tick_at: "2026-01-01T00:00:00Z",
            intents: &[],
            rejected: &[],
            effects: std::slice::from_ref(&effect),
            arrivals: &[],
            fast_forward: false,
            tool_trace: &[],
        });
        let encoded = spookery_kernel::store::codec::emit_effect(&effect);
        assert!(line.contains(&encoded), "{line}");
        assert!(!line.contains("\"payload\""));
    }
}
