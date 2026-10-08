use crate::seed::shuffle_key;
use spookery_kernel::intent::{ActorKind, Intent, Target};
use spookery_kernel::model::Now;

pub fn prioritize(
    now: &Now,
    mut intents: Vec<Intent>,
    tick_index: u64,
    shard_id: &str,
) -> Vec<Intent> {
    intents.sort_by(|left, right| {
        priority_key(now, left, tick_index, shard_id)
            .cmp(&priority_key(now, right, tick_index, shard_id))
    });
    intents
}

fn priority_key(
    now: &Now,
    intent: &Intent,
    tick_index: u64,
    shard_id: &str,
) -> (u8, u8, u8, [u8; 32]) {
    let kind = match intent.actor_kind {
        ActorKind::Player => 0,
        ActorKind::Npc => 1,
        ActorKind::System => 2,
    };
    let pc_clock = if targets_pc_clock(now, intent) { 0 } else { 1 };
    let claimed = if claimed(now, intent) { 0 } else { 1 };
    let shuffle = shuffle_key(shard_id, tick_index, &intent.actor_id);
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
