//! Write the round line and commit a live round.

use spookery_kernel::api::draft::Draft;
use spookery_kernel::api::journal::TickRecord;
use spookery_kernel::api::world::World;
use spookery_kernel::api::Reason;

use crate::play::{RoundCause, RoundOutput};
use crate::seed::round_seed;
use crate::state::Round;

pub(crate) fn run(
    world: &mut World,
    round: &mut Round,
    draft: Draft,
) -> Result<RoundOutput, Reason> {
    round.steps_run.push("record".to_string());
    if round.cause != RoundCause::Flush {
        round.steps_run.push("commit_round".to_string());
    }
    let record = round.recorded.is_none().then(|| TickRecord {
        tick_index: round.tick_index,
        rules_rev: spookery_kernel::api::store::RULES_REV,
        state_rev_before: round.rev_before,
        state_rev_after: round.rev_before + 1,
        world_time_before: round.world_time_before,
        world_time_after: round.world_time_after,
        presence_humans: round.humans,
        actors_present: world.presence().actors_present.clone(),
        seed: round_seed(&round.world_id, round.tick_index),
        last_tick_at: round.now_wall.clone(),
        intents: std::mem::take(&mut round.applied),
        rejected: std::mem::take(&mut round.rejected),
        effects: round.effects.clone(),
        arrivals: std::mem::take(&mut round.arrivals),
        fast_forward: round.fast_forward,
        dice: round.dice.clone(),
        steps_run: trim_steps(&round.steps_run),
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
        output.outcome = Some(spookery_kernel::api::commit::commit(world, draft, record)?);
    } else {
        spookery_kernel::api::commit::keep_in_memory(world, draft)?;
    }
    Ok(output)
}

/// Steps every round pushes, in order. `commit_round` may follow `record`.
const STANDARD_STEPS: &[&str] = &[
    "advance",
    "complete",
    "gather",
    "session_status",
    "resolve",
    "apply",
    "run_triggers",
    "observe",
    "record",
];

/// The part of `steps_run` worth storing. A list that skips or reorders the
/// standard steps is kept whole, so a broken round stays visible.
fn trim_steps(steps: &[String]) -> Vec<String> {
    let mut index = 0usize;
    let mut extras = Vec::new();
    let mut commit_seen = false;
    for step in steps {
        let name = step.as_str();
        if index < STANDARD_STEPS.len() && name == STANDARD_STEPS[index] {
            index += 1;
            continue;
        }
        if name == "commit_round" && index == STANDARD_STEPS.len() && !commit_seen {
            commit_seen = true;
            continue;
        }
        if name == "commit_round" || STANDARD_STEPS.contains(&name) {
            return steps.to_vec();
        }
        extras.push(step.clone());
    }
    if index < STANDARD_STEPS.len() {
        steps.to_vec()
    } else {
        extras
    }
}

#[cfg(test)]
mod tests {
    use spookery_kernel::api::effect::{Effect, EffectKind};
    use spookery_kernel::api::intent::ViewerId;
    use spookery_kernel::api::journal::{emit_entry, JournalEntry, TickRecord};

    use super::{trim_steps, STANDARD_STEPS};

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
        let line = emit_entry(&JournalEntry::Tick(TickRecord {
            tick_index: 1,
            rules_rev: spookery_kernel::api::store::RULES_REV,
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
        let encoded = spookery_kernel::api::store::codec::emit_effect(&effect);
        assert!(line.contains(&encoded), "{line}");
        assert!(!line.contains("\"payload\""));
    }

    #[test]
    fn a_standard_order_stores_no_steps() {
        let mut steps = STANDARD_STEPS
            .iter()
            .map(|step| step.to_string())
            .collect::<Vec<_>>();
        assert!(trim_steps(&steps).is_empty());
        steps.push("commit_round".to_string());
        assert!(trim_steps(&steps).is_empty());
    }

    #[test]
    fn extra_steps_are_the_only_steps_stored() {
        let steps = [
            "advance",
            "complete",
            "gather",
            "session_status",
            "resolve",
            "apply",
            "apply_effect",
            "cast:mira/leave_door_note",
            "run_triggers",
            "observe",
            "record",
            "commit_round",
        ]
        .map(String::from);
        assert_eq!(
            trim_steps(&steps),
            ["apply_effect", "cast:mira/leave_door_note"]
        );
    }

    #[test]
    fn reordered_steps_are_stored_whole() {
        let reordered = ["complete".to_string()];
        assert_eq!(trim_steps(&reordered), reordered);
        let partial = ["advance".to_string(), "apply_effect".to_string()];
        assert_eq!(trim_steps(&partial), partial);
    }
}
