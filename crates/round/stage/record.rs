//! Write the round line, and the commit when this round owns it.

use spookery_kernel::api::draft::Draft;
use spookery_kernel::api::effect::Effect;
use spookery_kernel::api::intent::Intent;
use spookery_kernel::api::journal::{self, Arrival, Rejected, TickRecord};
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
    if round.owns_commit {
        round.steps_run.push("commit_round".to_string());
    }
    let seed = round_seed(&round.world_id, round.tick_index);
    let actors = world.presence().actors_present.clone();
    let jsonl = render_round_line(&RoundLine {
        tick_index: round.tick_index,
        rules_rev: spookery_kernel::api::store::RULES_REV,
        state_rev_before: round.rev_before,
        state_rev_after: round.rev_before + 1,
        world_time_before: round.world_time_before,
        world_time_after: round.world_time_after,
        presence_humans: round.humans,
        actors_present: &actors,
        seed: &seed,
        last_tick_at: &round.now_wall,
        intents: &round.applied,
        rejected: &round.rejected,
        effects: &round.effects,
        arrivals: &round.arrivals,
        fast_forward: round.fast_forward,
        steps_run: &round.steps_run,
        dice: &round.dice,
    });
    let store = if round.recorded.is_some() {
        spookery_kernel::api::commit::RoundStore::Memory
    } else if round.cause == RoundCause::Flush {
        spookery_kernel::api::commit::RoundStore::Hold
    } else {
        spookery_kernel::api::commit::RoundStore::Write
    };
    let output = RoundOutput {
        reports: std::mem::take(&mut round.move_reports),
        dice: std::mem::take(&mut round.dice),
        summary: std::mem::take(&mut round.summary),
        lines: std::mem::take(&mut round.lines),
        seen: std::mem::take(&mut round.seen),
    };
    spookery_kernel::api::commit::finish_round(
        world,
        draft,
        store,
        jsonl,
        round.recorded.is_some(),
        output.summary.clone(),
    )?;
    Ok(output)
}

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
    pub steps_run: &'a [String],
    pub dice: &'a [String],
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
        dice: line.dice.to_vec(),
        steps_run: trim_steps(line.steps_run),
        memories: Vec::new(),
        now_hash: String::new(),
        prev: String::new(),
        chain: String::new(),
    };
    journal::emit_entry(&journal::JournalEntry::Tick(record))
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

    use super::{render_round_line, RoundLine, STANDARD_STEPS};

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
            rules_rev: spookery_kernel::api::store::RULES_REV,
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
            steps_run: &[],
            dice: &[],
        });
        let encoded = spookery_kernel::api::store::codec::emit_effect(&effect);
        assert!(line.contains(&encoded), "{line}");
        assert!(!line.contains("\"payload\""));
    }

    #[test]
    fn a_standard_order_omits_steps_run() {
        for commit in [true, false] {
            let line = render(&list_standard_steps(commit));
            assert!(!line.contains("steps_run"), "{line}");
        }
    }

    #[test]
    fn extra_steps_are_the_only_steps_run() {
        let mut steps = Vec::new();
        for name in [
            "advance",
            "complete",
            "gather",
            "session_status",
            "resolve",
            "perceive:mira",
            "apply",
            "apply_effect",
            "cast:mira/leave_door_note",
            "run_triggers",
            "observe",
            "record",
            "commit_round",
        ] {
            steps.push(name.to_string());
        }
        let line = render(&steps);
        assert!(line.contains(
            "\"steps_run\":[\"perceive:mira\",\"apply_effect\",\"cast:mira/leave_door_note\"]"
        ));
        assert!(!line.contains("advance"), "{line}");
    }

    #[test]
    fn reordered_steps_are_stored_whole() {
        let line = render(&["complete".to_string()]);
        assert!(line.contains("\"steps_run\":[\"complete\"]"), "{line}");
        let partial = render(&["advance".to_string(), "perceive:mira".to_string()]);
        assert!(
            partial.contains("\"steps_run\":[\"advance\",\"perceive:mira\"]"),
            "{partial}"
        );
    }

    fn list_standard_steps(commit: bool) -> Vec<String> {
        let mut steps = STANDARD_STEPS
            .iter()
            .map(|step| step.to_string())
            .collect::<Vec<_>>();
        if commit {
            steps.push("commit_round".to_string());
        }
        steps
    }

    fn render(steps: &[String]) -> String {
        render_round_line(&RoundLine {
            tick_index: 1,
            rules_rev: spookery_kernel::api::store::RULES_REV,
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
            effects: &[],
            arrivals: &[],
            fast_forward: false,
            steps_run: steps,
            dice: &[],
        })
    }
}
