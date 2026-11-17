//! Write the round line and commit a live round.

use crate::session;
use crate::store::records::RoundRecord;
use crate::store::{commit, RULES_REV};
use crate::Reason;

use crate::ecs::Presence;
use crate::round::play::{RoundOutput, StagedRound};
use crate::round::stage::deliver::Deliveries;
use crate::round::Ledger;

/// What the round showed. A live round adds its commit outcome.
pub(crate) fn take_output(ledger: &mut Ledger, delivered: Deliveries) -> RoundOutput {
    RoundOutput {
        reports: std::mem::take(&mut ledger.reports),
        dice: ledger.dice.clone(),
        changed_keys: delivered.changed_keys,
        lines: delivered.lines,
        seen: delivered.seen,
        effects: ledger.effects.clone(),
        commit: None,
    }
}

/// Write the round line and commit it. The session owns the pack lock.
pub(crate) fn run(
    session: &mut session::Session,
    staged: StagedRound,
    presence: &Presence,
    wall: &str,
    fast_forward: bool,
) -> Result<RoundOutput, Reason> {
    let StagedRound {
        world,
        clock,
        mut ledger,
        delivered,
    } = staged;
    let mut output = take_output(&mut ledger, delivered);
    let record = RoundRecord {
        round_index: clock.round_index,
        rules_rev: RULES_REV,
        state_rev_before: 0,
        state_rev_after: 0,
        world_time_before: clock.world_time_before,
        world_time_after: clock.world_time_after,
        presence_players: presence.players,
        actors_present: presence.actors_present.clone(),
        last_round_at: wall.to_string(),
        intents: session.queued_intents().to_vec(),
        refused: ledger.refused,
        effects: ledger.effects,
        fast_forward,
        dice: ledger.dice,
        extra_steps: ledger.extra_steps,
        memories: Vec::new(),
        state_hash: String::new(),
        prev: String::new(),
        chain: String::new(),
    };
    output.commit = Some(commit::commit(session, world, record)?);
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
                place: Some("docks_shop".into()),
                id: "lamp_oil".to_string(),
                delta: -1,
                max: None,
            },
            viewer: Some(Viewer::Npc("mira".into())),
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
