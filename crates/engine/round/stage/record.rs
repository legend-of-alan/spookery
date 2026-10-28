//! Write the round line and commit a live round.

use crate::draft::Draft;
use crate::journal::RoundRecord;
use crate::world::World;
use crate::Reason;

use crate::round::play::RoundOutput;
use crate::round::state::round_seed;
use crate::round::state::Round;

pub(crate) fn run(
    world: &mut World,
    round: &mut Round,
    draft: Draft,
) -> Result<RoundOutput, Reason> {
    let record = round.replayed.is_none().then(|| RoundRecord {
        round_index: round.round_index,
        rules_rev: crate::store::RULES_REV,
        state_rev_before: 0,
        state_rev_after: 0,
        world_time_before: round.world_time_before,
        world_time_after: round.world_time_after,
        presence_humans: draft.now().presence.humans,
        actors_present: draft.now().presence.actors_present.clone(),
        seed: round_seed(&round.world_id, round.round_index),
        last_tick_at: round.now_wall.clone(),
        intents: std::mem::take(&mut round.applied),
        rejected: std::mem::take(&mut round.rejected),
        effects: round.effects.clone(),
        arrivals: std::mem::take(&mut round.arrivals),
        fast_forward: round.fast_forward,
        dice: round.dice.clone(),
        steps_run: std::mem::take(&mut round.steps_run),
        memories: Vec::new(),
        now_hash: String::new(),
        prev: String::new(),
        chain: String::new(),
    });
    let mut output = RoundOutput {
        reports: std::mem::take(&mut round.move_reports),
        dice: std::mem::take(&mut round.dice),
        summary: std::mem::take(&mut round.summary),
        lines: std::mem::take(&mut round.lines),
        seen: std::mem::take(&mut round.seen),
        effects: std::mem::take(&mut round.effects),
        outcome: None,
    };
    if let Some(record) = record {
        output.outcome = Some(crate::commit::commit(world, draft, record)?);
    } else {
        crate::commit::keep_in_memory(world, draft)?;
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use crate::effect::{Effect, EffectKind};
    use crate::intent::ViewerId;
    use crate::journal::{emit_entry, JournalEntry, RoundRecord};

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
        let line = emit_entry(&JournalEntry::Round(RoundRecord {
            round_index: 1,
            rules_rev: crate::store::RULES_REV,
            state_rev_before: 0,
            state_rev_after: 1,
            world_time_before: 0,
            world_time_after: 60,
            presence_humans: 0,
            actors_present: Vec::new(),
            seed: "seed".to_string(),
            last_tick_at: "2026-01-01T00:00:00Z".to_string(),
            intents: Vec::new(),
            rejected: Vec::new(),
            effects: vec![effect.clone()],
            arrivals: Vec::new(),
            fast_forward: false,
            dice: Vec::new(),
            steps_run: Vec::new(),
            memories: Vec::new(),
            now_hash: String::new(),
            prev: String::new(),
            chain: String::new(),
        }));
        let encoded = crate::store::codec::emit_effect(&effect);
        assert!(line.contains(&encoded), "{line}");
        assert!(!line.contains("\"payload\""));
    }
}
