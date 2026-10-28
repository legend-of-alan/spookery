//! Which habits an NPC may run on this tick.

use crate::canon::Canon;
use crate::intent::{kernel_intent_id, Intent, Source, Target, ViewerId};
use crate::model::Now;
use crate::rules::cond;
use crate::rules::expr::Ctx;

pub(crate) fn habits_on(canon: &Canon, now: &Now, actor_id: &str) -> Vec<String> {
    canon
        .rules
        .habits_of(actor_id)
        .into_iter()
        .filter(|rule| {
            let intent = mint(now, actor_id, &rule.local_id);
            cond::holds(canon, now, &Ctx { intent: &intent }, &rule.if_).is_ok()
        })
        .map(|rule| rule.local_id.clone())
        .collect()
}

pub(crate) fn is_eligible(canon: &Canon, now: &Now, npc_id: &str) -> bool {
    let Some(actor) = now.actors.get(npc_id) else {
        return false;
    };
    if !actor.is_npc() {
        return false;
    }
    let Some(profile) = canon.profile(npc_id) else {
        return false;
    };
    if !profile.eligible {
        return false;
    }
    if actor.is_busy(now.world_time) || player_present_at(now, actor.place()) {
        return false;
    }
    let counters = actor.act();
    if counters.last_act_round == now.round_index {
        return false;
    }
    if now.presence.humans == 0 {
        u64::from(counters.idle_acts_today) < canon.budget.idle_acts_per_world_day
    } else {
        u64::from(counters.live_acts_this_hour) < canon.budget.live_acts_per_world_hour
    }
}

fn player_present_at(now: &Now, place: &str) -> bool {
    now.presence.actors_present.iter().any(|id| {
        now.actors
            .get(id)
            .is_some_and(|actor| actor.is_pc() && actor.place() == place)
    })
}

pub(crate) fn mint(now: &Now, actor_id: &str, local_id: &str) -> Intent {
    Intent::new(
        kernel_intent_id(&now.shard_id, now.round_index, actor_id, local_id),
        now.round_index,
        ViewerId::Npc(actor_id.to_string()),
        Source::Kernel,
        local_id,
        Target::Empty,
    )
}
