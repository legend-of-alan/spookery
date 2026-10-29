//! Write the round line and commit a live round.

use crate::draft::Draft;
use crate::world::World;
use crate::Reason;

use crate::round::dice::round_seed;
use crate::round::log::{RoundLine, RoundLog};
use crate::round::play::RoundOutput;
use crate::round::stage::advance::Clock;
use crate::round::stage::deliver::Delivered;

/// What the round showed. A live round adds its commit outcome.
pub(crate) fn take_output(log: &mut RoundLog, delivered: Delivered) -> RoundOutput {
    RoundOutput {
        reports: std::mem::take(&mut log.reports),
        dice: log.dice.clone(),
        summary: delivered.summary,
        lines: delivered.lines,
        seen: delivered.seen,
        effects: log.effects.clone(),
        outcome: None,
    }
}

/// Write the round line and commit it. The World owns the pack lock.
pub(crate) fn commit_round(
    world: &mut World,
    draft: Draft,
    wall: &str,
    clock: Clock,
    mut log: RoundLog,
    delivered: Delivered,
    fast_forward: bool,
) -> Result<RoundOutput, Reason> {
    let mut output = take_output(&mut log, delivered);
    let line = RoundLine {
        presence: draft.presence.clone(),
        seed: round_seed(&draft.now().shard_id, clock.round_index),
        clock,
        wall: wall.to_string(),
        fast_forward,
        log,
        queue: world.queued_intents().to_vec(),
    };
    output.outcome = Some(crate::commit::commit(world, draft, line)?);
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
            last_round_at: "2026-01-01T00:00:00Z".to_string(),
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
