//! Which habits an NPC may run this round.

use crate::canon::Canon;
use crate::intent::{kernel_intent_id, Intent, Source, Target, ViewerId};
use crate::model::{Now, Presence};
use crate::rule::When;
use crate::rules::cond;
use crate::rules::expand::{expand_outcome, stamp, PreviewDice};
use crate::rules::expr::Ctx;
use crate::rules::gate;

pub(crate) fn habit_intents(canon: &Canon, now: &Now, presence: &Presence) -> Vec<Intent> {
    let mut intents = Vec::new();
    for actor_id in now.actors.keys() {
        if !is_eligible(canon, now, presence, actor_id) {
            continue;
        }
        if let Some(intent) = select_one(canon, now, presence, actor_id) {
            intents.push(intent);
        }
    }
    intents
}

fn select_one(canon: &Canon, now: &Now, presence: &Presence, actor_id: &str) -> Option<Intent> {
    for rule in canon.rules.habits_of(actor_id) {
        if !matches!(rule.when, When::Habit) {
            continue;
        }
        let intent = mint(now, actor_id, &rule.local_id);
        let ctx = Ctx {
            intent: &intent,
            presence,
        };
        if cond::holds(canon, now, &ctx, &rule.if_).is_err() {
            continue;
        }
        if gate::gate(canon, now, presence, &intent).is_err() {
            continue;
        }
        match expand_outcome(canon, now, presence, &intent, None, &mut PreviewDice) {
            Ok(effects) => {
                let mut intent = intent;
                stamp(&mut intent, &effects);
                return Some(intent);
            }
            Err(_) => continue,
        }
    }
    None
}

pub(crate) fn habits_on(
    canon: &Canon,
    now: &Now,
    presence: &Presence,
    actor_id: &str,
) -> Vec<String> {
    canon
        .rules
        .habits_of(actor_id)
        .into_iter()
        .filter(|rule| {
            let intent = mint(now, actor_id, &rule.local_id);
            let ctx = Ctx {
                intent: &intent,
                presence,
            };
            cond::holds(canon, now, &ctx, &rule.if_).is_ok()
        })
        .map(|rule| rule.local_id.clone())
        .collect()
}

pub(crate) fn is_eligible(canon: &Canon, now: &Now, presence: &Presence, npc_id: &str) -> bool {
    let Some(actor) = now.actors.get(npc_id) else {
        return false;
    };
    if !actor.is_npc() {
        return false;
    }
    let Some(profile) = canon.idle_profile(npc_id) else {
        return false;
    };
    if !profile.eligible {
        return false;
    }
    if actor.is_busy(now.world_time) || player_present_at(now, presence, actor.place()) {
        return false;
    }
    let counters = actor.act();
    if counters.last_act_round == now.round_index {
        return false;
    }
    if presence.humans == 0 {
        u64::from(counters.idle_acts_today) < canon.budget.idle_acts_per_world_day
    } else {
        u64::from(counters.live_acts_this_hour) < canon.budget.live_acts_per_world_hour
    }
}

fn player_present_at(now: &Now, presence: &Presence, place: &str) -> bool {
    presence.actors_present.iter().any(|id| {
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
