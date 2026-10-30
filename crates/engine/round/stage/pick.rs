//! Take queued intents and habits, in the order this round will try them.

use std::collections::BTreeSet;

use crate::canon::Canon;
use crate::draft::Draft;
use crate::expand::stamp_for_priority;
use crate::intent::{Intent, Target, Viewer};
use crate::model::{Presence, State};
use crate::rules::habits::{habit_intents, is_player_at};
use crate::travel::find_exit;
use crate::Reason;

use crate::round::dice::shuffle_key;

/// The queue stamped, with habits, in priority order, cut to the budget.
pub(crate) fn run(draft: &Draft, queue: &[Intent]) -> Result<Vec<Intent>, Reason> {
    let mut candidates = queue.to_vec();
    for intent in &mut candidates {
        stamp_for_priority(draft.canon(), draft.state(), draft.presence, intent);
    }
    candidates.extend(habit_intents(draft.canon(), draft.state(), draft.presence));
    let (canon, now, presence) = (draft.canon(), draft.state(), draft.presence);
    candidates.sort_by_cached_key(|intent| priority(canon, now, presence, intent));
    candidates.truncate(draft.budget().max_intents_applied as usize);
    check_unique_ids(candidates)
}

fn check_unique_ids(candidates: Vec<Intent>) -> Result<Vec<Intent>, Reason> {
    let mut seen = BTreeSet::new();
    for intent in &candidates {
        if !seen.insert(intent.id.clone()) {
            return Err(Reason::Schema);
        }
    }
    Ok(candidates)
}

/// Who acts first: players, then a PC's clock, then a claimed target, then the shuffle.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct Priority {
    actor: ActorRank,
    clock: ClockRank,
    claim: SeatedRank,
    shuffle: [u8; 32],
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum ActorRank {
    Player,
    Npc,
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum ClockRank {
    Pc,
    Other,
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum SeatedRank {
    Claimed,
    Unclaimed,
}

fn priority(canon: &Canon, now: &State, presence: &Presence, intent: &Intent) -> Priority {
    let actor = match intent.actor {
        Viewer::Player(_) => ActorRank::Player,
        Viewer::Npc(_) => ActorRank::Npc,
    };
    let clock = if targets_pc_clock(now, intent) {
        ClockRank::Pc
    } else {
        ClockRank::Other
    };
    let claim = if touches_seated_player(canon, now, presence, intent) {
        SeatedRank::Claimed
    } else {
        SeatedRank::Unclaimed
    };
    // Player lines that share a priority keep the queue order. NPC ties still shuffle.
    let shuffle = if actor == ActorRank::Player {
        [0u8; 32]
    } else {
        shuffle_key(&now.world_id, now.round_index, intent.actor.actor_id())
    };
    Priority {
        actor,
        clock,
        claim,
        shuffle,
    }
}

fn targets_pc_clock(now: &State, intent: &Intent) -> bool {
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

fn touches_seated_player(canon: &Canon, now: &State, presence: &Presence, intent: &Intent) -> bool {
    let present = &presence.actors_present;
    let target_hit = match &intent.target {
        Target::Actor(id) | Target::Item(id) => present.iter().any(|actor| actor == id),
        Target::Place(place) => is_player_at(now, presence, place),
        Target::Exit(name) => now
            .actors
            .get(intent.actor.actor_id())
            .and_then(|actor| find_exit(canon, actor.place(), name))
            .is_some_and(|exit| is_player_at(now, presence, &exit.to)),
        _ => false,
    };
    let message_hit = intent
        .args
        .message_to
        .as_ref()
        .is_some_and(|ids| ids.iter().any(|id| present.iter().any(|actor| actor == id)));
    target_hit || message_hit
}
