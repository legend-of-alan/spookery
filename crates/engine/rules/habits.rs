//! Which habits an NPC may run this round.

use crate::canon::Canon;
use crate::intent::{Intent, Target, Viewer};
use crate::model::{ActorLive, Presence, State};
use crate::rules::cond;
use crate::rules::context::Context;
use crate::rules::expand;
use crate::rules::gate;

/// One habit intent per NPC whose habit the gate admits, its target not yet stamped.
pub(crate) fn intents(canon: &Canon, state: &State, presence: &Presence) -> Vec<Intent> {
    let actors = state.actors.keys();
    let intents = actors.filter_map(|actor_id| intent_of(canon, state, presence, actor_id));
    intents.collect()
}

/// The first habit, in file order, that the gate admits and whose effects expand.
fn intent_of(canon: &Canon, state: &State, presence: &Presence, actor_id: &str) -> Option<Intent> {
    let habits = canon.rules.habits_of(actor_id);
    habits.into_iter().find_map(|rule| {
        let intent = Intent::habit(state, actor_id, &rule.local_id);
        let context = Context::new(canon, state, presence, &intent);
        let runs = gate::admit(&context, rule).is_ok() && expand::preview(&context, rule).is_ok();
        runs.then_some(intent)
    })
}

/// This NPC's habits whose `if` holds, for its view.
pub(crate) fn list_whose_if_holds(
    canon: &Canon,
    state: &State,
    presence: &Presence,
    npc_id: &str,
) -> Vec<String> {
    let npc = Viewer::Npc(npc_id.to_string());
    let habits = canon.rules.habits_of(npc_id).into_iter();
    habits
        .filter(|rule| {
            let (verb, conds) = (&rule.local_id, &rule.if_);
            cond::if_holds(canon, state, presence, &npc, verb, Target::Empty, conds)
        })
        .map(|rule| rule.local_id.clone())
        .collect()
}

pub(crate) fn is_eligible(canon: &Canon, state: &State, presence: &Presence, npc_id: &str) -> bool {
    let Some(actor) = state.actors.get(npc_id) else {
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
    if actor.is_busy(state.world_time) || is_player_at(state, presence, &actor.place) {
        return false;
    }
    let counters = &actor.act;
    if counters.last_act_round == state.round_index {
        return false;
    }
    if presence.players == 0 {
        u64::from(counters.idle_acts_today) < canon.budget.idle_acts_per_world_day
    } else {
        u64::from(counters.live_acts_this_hour) < canon.budget.live_acts_per_world_hour
    }
}

/// A seated player's PC is in `place`.
pub(crate) fn is_player_at(state: &State, presence: &Presence, place: &str) -> bool {
    state.actors_at(place).iter().any(|id| {
        presence.actors_present.contains(id) && state.actors.get(id).is_some_and(ActorLive::is_pc)
    })
}
