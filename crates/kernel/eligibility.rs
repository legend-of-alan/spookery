use crate::canon::Canon;
use crate::model::Now;

pub fn is_eligible(canon: &Canon, now: &Now, npc_id: &str) -> bool {
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
    if actor.busy() || player_present_at(now, actor.place()) {
        return false;
    }
    let Some(tick) = actor.tick() else {
        return false;
    };
    if tick.last_act_tick == now.tick_index {
        return false;
    }
    if now.presence.humans == 0 {
        u64::from(tick.idle_acts_today) < canon.budget.idle_acts_per_world_day
    } else {
        u64::from(tick.live_acts_this_hour) < canon.budget.live_acts_per_world_hour
    }
}

fn player_present_at(now: &Now, place: &str) -> bool {
    now.presence.actors_present.iter().any(|id| {
        now.actors
            .get(id)
            .is_some_and(|actor| actor.is_pc() && actor.place() == place)
    })
}
