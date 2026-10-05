use spookery_kernel::canon::{Canon, Tier};
use spookery_kernel::intent::{ActorKind, Intent, Target};
use spookery_kernel::model::Now;
use crate::seed::shuffle_key;

pub fn prioritize(
    canon: &Canon,
    now: &Now,
    mut intents: Vec<Intent>,
    tick_index: u64,
    shard_id: &str,
) -> Vec<Intent> {
    intents.sort_by(|left, right| {
        priority_key(canon, now, left, tick_index, shard_id)
            .cmp(&priority_key(canon, now, right, tick_index, shard_id))
    });
    intents
}

fn priority_key(
    canon: &Canon,
    now: &Now,
    intent: &Intent,
    tick_index: u64,
    shard_id: &str,
) -> (u8, u8, u8, u8, [u8; 32]) {
    let kind = match intent.actor_kind {
        ActorKind::Player => 0,
        ActorKind::Npc => 1,
        ActorKind::System => 2,
    };
    let pc_clock = if targets_pc_clock(now, intent) { 0 } else { 1 };
    let claimed = if claimed(now, intent) { 0 } else { 1 };
    let tier = if notable(canon, intent) { 0 } else { 1 };
    let shuffle = shuffle_key(shard_id, tick_index, &intent.actor_id);
    (kind, pc_clock, claimed, tier, shuffle)
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

fn notable(canon: &Canon, intent: &Intent) -> bool {
    canon.profile(&intent.actor_id).is_some_and(|profile| {
        matches!(profile.tier, Tier::Named | Tier::Major) || profile.plot_relevant
    })
}
