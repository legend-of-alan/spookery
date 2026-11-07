//! Write the round line and commit a live round.

use crate::store::records::RoundRecord;
use crate::store::{commit, RULES_REV};
use crate::world::World;
use crate::Reason;

use crate::round::dice::round_seed;
use crate::round::log::RoundLog;
use crate::round::play::{RoundOutput, StagedRound};
use crate::round::stage::deliver::Deliveries;

/// What the round showed. A live round adds its commit outcome.
pub(crate) fn take_output(log: &mut RoundLog, delivered: Deliveries) -> RoundOutput {
    RoundOutput {
        reports: std::mem::take(&mut log.reports),
        dice: log.dice.clone(),
        changed_keys: delivered.changed_keys,
        lines: delivered.lines,
        seen: delivered.seen,
        effects: log.effects.clone(),
        commit: None,
    }
}

/// Write the round line and commit it. The World owns the pack lock.
pub(crate) fn run(
    world: &mut World,
    staged: StagedRound,
    wall: &str,
    fast_forward: bool,
) -> Result<RoundOutput, Reason> {
    let StagedRound {
        state,
        clock,
        mut log,
        delivered,
    } = staged;
    let mut output = take_output(&mut log, delivered);
    let record = RoundRecord {
        round_index: clock.round_index,
        rules_rev: RULES_REV,
        state_rev_before: 0,
        state_rev_after: 0,
        world_time_before: clock.world_time_before,
        world_time_after: clock.world_time_after,
        presence_players: state.presence().players,
        actors_present: state.presence().actors_present.clone(),
        seed: round_seed(&state.world_id, clock.round_index),
        last_round_at: wall.to_string(),
        intents: world.queued_intents().to_vec(),
        refused: log.refused,
        effects: log.effects,
        fast_forward,
        dice: log.dice,
        extra_steps: log.extra_steps,
        memories: Vec::new(),
        state_hash: String::new(),
        prev: String::new(),
        chain: String::new(),
    };
    output.commit = Some(commit::commit(world, state, record)?);
    Ok(output)
}

#[cfg(test)]
mod tests {
    use crate::intent::Viewer;
    use crate::rules::effect::{Effect, EffectKind};
    use crate::store::journal::emit_record;
    use crate::store::records::{Record, RoundRecord};

    #[test]
    fn round_record_embeds_the_flat_effect() {
        let effect = Effect {
            kind: EffectKind::Item {
                actor_id: None,
                place: Some("docks_shop".to_string()),
                id: "lamp_oil".to_string(),
                delta: -1,
                max: None,
            },
            viewer: Some(Viewer::Npc("mira".to_string())),
            intent_id: "i".to_string(),
        };
        let line = emit_record(&Record::Round(RoundRecord {
            effects: vec![effect.clone()],
            ..Default::default()
        }));
        let encoded = crate::store::records::emit_effect(&effect);
        assert!(line.contains(&encoded), "{line}");
        assert!(!line.contains("\"payload\""));
    }
}
