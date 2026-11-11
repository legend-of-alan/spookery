//! Which habits an NPC may run this round.

use crate::canon::Canon;
use crate::intent::{Intent, Target, Viewer};
use crate::rules::cond;
use crate::rules::context::Context;
use crate::rules::expand;
use crate::rules::gate;
use crate::state::id::ActorId;
use crate::state::{ActorKind, Presence, State};

/// One habit intent per NPC whose habit the gate admits, its target not yet stamped.
pub(crate) fn intents(canon: &Canon, state: &State, presence: &Presence) -> Vec<Intent> {
    let actors = state.actors().kind.keys();
    let intents = actors.filter_map(|actor_id| intent_of(canon, state, presence, actor_id));
    intents.collect()
}

/// For an NPC the engine may move now, the first habit, in file order, that the gate admits
/// and whose effects expand.
fn intent_of(
    canon: &Canon,
    state: &State,
    presence: &Presence,
    actor_id: &ActorId,
) -> Option<Intent> {
    if !is_eligible(canon, state, presence, actor_id) {
        return None;
    }
    let habits = canon.rules.habits_of(actor_id);
    habits.into_iter().find_map(|rule| {
        let intent = Intent::habit(state, actor_id, &rule.local_id);
        let context = Context::new(canon, state, presence, &intent);
        let runs =
            gate::admit(&context, rule, None).is_ok() && expand::preview(&context, rule).is_ok();
        runs.then_some(intent)
    })
}

/// This NPC's habits whose `if` holds, for its view.
pub(crate) fn list_whose_if_holds(
    canon: &Canon,
    state: &State,
    presence: &Presence,
    npc_id: &ActorId,
) -> Vec<String> {
    let npc = Viewer::Npc(npc_id.clone());
    let habits = canon.rules.habits_of(npc_id).into_iter();
    habits
        .filter(|rule| {
            let (verb, conds) = (&rule.local_id, &rule.if_);
            cond::if_holds(canon, state, presence, &npc, verb, Target::Empty, conds)
        })
        .map(|rule| rule.local_id.clone())
        .collect()
}

/// Whether the engine may run one of this NPC's habits on its own this round: its idle
/// profile allows it, it is free, no seated player is with it, and its act window has room.
fn is_eligible(canon: &Canon, state: &State, presence: &Presence, npc_id: &ActorId) -> bool {
    if state.actors().kind.get(npc_id) != Some(&ActorKind::Npc) {
        return false;
    }
    let Some(profile) = canon.idle_profile(npc_id) else {
        return false;
    };
    if !profile.eligible {
        return false;
    }
    let (Some(counters), Some(place)) = (
        state.actors().acts.get(npc_id),
        state.actors().place.get(npc_id),
    ) else {
        return false;
    };
    if counters.is_busy(state.world_time) || is_player_at(state, presence, place) {
        return false;
    }
    if counters.last_act_round == state.round_index {
        return false;
    }
    if presence.players == 0 {
        u64::from(counters.idle_acts_this_window) < canon.budget.idle_acts_per_window
    } else {
        u64::from(counters.live_acts_this_window) < canon.budget.live_acts_per_window
    }
}

/// A seated player's PC is in `place`.
pub(crate) fn is_player_at(state: &State, presence: &Presence, place: &str) -> bool {
    state.actors_at(place).iter().any(|id| {
        presence.actors_present.contains(id) && state.actors().kind.get(id) == Some(&ActorKind::Pc)
    })
}
