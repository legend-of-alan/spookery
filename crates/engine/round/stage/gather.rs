//! Take queued intents and habits, in the order this round will try them.

use std::collections::BTreeSet;

use crate::draft::Draft;
use crate::expand::{habit_intents, stamp_for_priority};
use crate::intent::{Intent, Target, ViewerId};
use crate::model::Now;
use crate::Reason;

use crate::round::dice::shuffle_key;

/// The queue stamped, with habits, in priority order, cut to the budget.
pub(crate) fn run(draft: &Draft, queue: &[Intent]) -> Result<Vec<Intent>, Reason> {
    let mut candidates = queue.to_vec();
    for intent in &mut candidates {
        stamp_for_priority(draft.canon(), draft.now(), intent);
    }
    candidates.extend(habit_intents(draft.canon(), draft.now()));
    let now = draft.now();
    let mut candidates = order_intents(now, candidates, now.round_index, &now.shard_id);
    let max_intents = draft.budget().max_intents_applied as usize;
    if candidates.len() > max_intents {
        candidates.truncate(max_intents);
    }
    unique(candidates)
}

fn unique(candidates: Vec<Intent>) -> Result<Vec<Intent>, Reason> {
    let mut seen = BTreeSet::new();
    for intent in &candidates {
        if !seen.insert(intent.id.clone()) {
            return Err(Reason::Schema);
        }
    }
    Ok(candidates)
}

fn order_intents(
    now: &Now,
    mut intents: Vec<Intent>,
    round_index: u64,
    world_id: &str,
) -> Vec<Intent> {
    intents.sort_by_cached_key(|intent| priority(now, intent, round_index, world_id));
    intents
}

/// Who acts first: players, then a PC's clock, then a claimed target, then the shuffle.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct Priority {
    actor: ActorRank,
    clock: ClockRank,
    claim: ClaimRank,
    shuffle: [u8; 32],
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum ActorRank {
    Player,
    Npc,
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum ClockRank {
    PcClock,
    Other,
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum ClaimRank {
    Claimed,
    Unclaimed,
}

fn priority(now: &Now, intent: &Intent, round_index: u64, world_id: &str) -> Priority {
    let actor = match intent.actor {
        ViewerId::Player(_) => ActorRank::Player,
        ViewerId::Npc(_) => ActorRank::Npc,
    };
    let clock = if targets_pc_clock(now, intent) {
        ClockRank::PcClock
    } else {
        ClockRank::Other
    };
    let claim = if claimed(now, intent) {
        ClaimRank::Claimed
    } else {
        ClaimRank::Unclaimed
    };
    // Player lines that share a priority keep the queue order. NPC ties still shuffle.
    let shuffle = if actor == ActorRank::Player {
        [0u8; 32]
    } else {
        shuffle_key(world_id, round_index, intent.actor.actor_id())
    };
    Priority {
        actor,
        clock,
        claim,
        shuffle,
    }
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
