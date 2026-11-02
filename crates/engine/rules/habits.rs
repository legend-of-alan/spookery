//! Which habits an NPC may run this round.

use crate::canon::Canon;
use crate::intent::{Intent, Target, Viewer};
use crate::model::{ActorLive, Presence, State};
use crate::rules::case::Case;
use crate::rules::cond;
use crate::rules::expand::PreviewDice;
use crate::rules::gate;

/// One habit intent per NPC whose habit the gate admits, its target not yet stamped.
pub(crate) fn intents(canon: &Canon, now: &State, presence: &Presence) -> Vec<Intent> {
    let actors = now.actors.keys();
    let intents = actors.filter_map(|actor_id| intent_of(canon, now, presence, actor_id));
    intents.collect()
}

/// The first habit, in file order, that the gate admits and whose effects expand.
fn intent_of(canon: &Canon, now: &State, presence: &Presence, actor_id: &str) -> Option<Intent> {
    let habits = canon.rules.habits_of(actor_id);
    habits.into_iter().find_map(|rule| {
        let intent = Intent::habit(now, actor_id, &rule.local_id);
        let case = Case::new(canon, now, presence, &intent);
        let runs =
            gate::admit(&case, rule).is_ok() && rule.expand(&case, None, &mut PreviewDice).is_ok();
        runs.then_some(intent)
    })
}

/// This NPC's habits whose `if` holds, for its view.
pub(crate) fn list_whose_if_holds(
    canon: &Canon,
    now: &State,
    presence: &Presence,
    npc_id: &str,
) -> Vec<String> {
    let npc = Viewer::Npc(npc_id.to_string());
    let habits = canon.rules.habits_of(npc_id).into_iter();
    habits
        .filter(|rule| {
            let (verb, conds) = (&rule.local_id, &rule.if_);
            cond::if_holds(canon, now, presence, &npc, verb, Target::Empty, conds)
        })
        .map(|rule| rule.local_id.clone())
        .collect()
}

pub(crate) fn is_eligible(canon: &Canon, now: &State, presence: &Presence, npc_id: &str) -> bool {
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
    if actor.is_busy(now.world_time) || is_player_at(now, presence, actor.place()) {
        return false;
    }
    let counters = actor.act();
    if counters.last_act_round == now.round_index {
        return false;
    }
    if presence.players == 0 {
        u64::from(counters.idle_acts_today) < canon.budget.idle_acts_per_world_day
    } else {
        u64::from(counters.live_acts_this_hour) < canon.budget.live_acts_per_world_hour
    }
}

/// A seated player's PC is in `place`.
pub(crate) fn is_player_at(now: &State, presence: &Presence, place: &str) -> bool {
    now.actors_at(place).iter().any(|id| {
        presence.actors_present.contains(id) && now.actors.get(id).is_some_and(ActorLive::is_pc)
    })
}
