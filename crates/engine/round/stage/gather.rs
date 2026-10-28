//! Take queued intents and habits, in the order this tick will try them.

use std::collections::BTreeSet;

use crate::draft::Draft;
use crate::expand::{habit_intents, stamp_for_priority};
use crate::intent::{ActorKind, Intent, Target};
use crate::model::Now;
use crate::Reason;

use crate::round::dice::shuffle_key;
use crate::round::state::Round;

pub(crate) fn run(round: &mut Round, draft: &mut Draft) -> Result<(), Reason> {
    round.opposing_candidates = match &round.replayed {
        Some(replayed) => replayed.clone(),
        None => round.queued.clone(),
    };
    let candidates = if let Some(replayed) = &round.replayed {
        replayed.clone()
    } else {
        let mut candidates = round.queued.clone();
        for intent in &mut candidates {
            stamp_for_priority(draft.canon(), draft.now(), intent);
        }
        candidates.extend(habit_intents(draft.canon(), draft.now()));
        let mut candidates =
            order_intents(draft.now(), candidates, round.round_index, &round.world_id);
        let max_intents = draft.budget().max_intents_applied as usize;
        if candidates.len() > max_intents {
            candidates.truncate(max_intents);
        }
        candidates
    };
    let mut seen = BTreeSet::new();
    for intent in &candidates {
        if !seen.insert(intent.id.clone()) {
            return Err(Reason::Schema);
        }
    }
    round.candidates = candidates;
    Ok(())
}

fn order_intents(
    now: &Now,
    mut intents: Vec<Intent>,
    round_index: u64,
    world_id: &str,
) -> Vec<Intent> {
    intents.sort_by(|left, right| {
        priority_key(now, left, round_index, world_id).cmp(&priority_key(
            now,
            right,
            round_index,
            world_id,
        ))
    });
    intents
}

fn priority_key(
    now: &Now,
    intent: &Intent,
    round_index: u64,
    world_id: &str,
) -> (u8, u8, u8, [u8; 32]) {
    let kind = match intent.actor_kind {
        ActorKind::Player => 0,
        ActorKind::Npc => 1,
        ActorKind::System => 2,
    };
    let pc_clock = if targets_pc_clock(now, intent) { 0 } else { 1 };
    let claimed = if claimed(now, intent) { 0 } else { 1 };
    // Player lines that share a priority keep the queue order. NPC ties still shuffle.
    let shuffle = if intent.actor_kind == ActorKind::Player {
        [0u8; 32]
    } else {
        shuffle_key(world_id, round_index, &intent.actor_id)
    };
    (kind, pc_clock, claimed, shuffle)
}

fn targets_pc_clock(now: &Now, intent: &Intent) -> bool {
    let Target::Clock(id) = &intent.target else {
        return false;
    };
    now.clocks.get(id).is_some_and(|clock| {
        clock
            .owners
            .iter()
            .any(|owner| now.actors.get(owner).is_some_and(|actor| actor.is_pc()))
    })
}

fn claimed(now: &Now, intent: &Intent) -> bool {
    let present = &now.presence.actors_present;
    let target_hit = match &intent.target {
        Target::Actor(id) => present.iter().any(|actor| actor == id),
        Target::Place(place) => present.iter().any(|actor| {
            now.actors
                .get(actor)
                .is_some_and(|actor| actor.is_pc() && actor.place() == place)
        }),
        _ => false,
    };
    let message_hit = intent
        .args
        .message_to
        .as_ref()
        .is_some_and(|ids| ids.iter().any(|id| present.iter().any(|actor| actor == id)));
    target_hit || message_hit
}
