//! Which habits an NPC may run this round.

use crate::canon::Canon;
use crate::ecs::id::ActorId;
use crate::ecs::{self, component, Presence};
use crate::intent::{Intent, Target};
use crate::rules::cond;
use crate::rules::context::Context;
use crate::rules::expand;
use crate::rules::gate;

/// One habit intent per NPC whose habit the gate admits, its target not yet stamped.
pub(crate) fn intents(canon: &Canon, world: &ecs::World, presence: &Presence) -> Vec<Intent> {
    let actors = world.actors().query::<ActorId>();
    let intents = actors
        .iter()
        .filter_map(|actor_id| intent_of(canon, world, presence, actor_id));
    intents.collect()
}

/// For an NPC the engine may move now, the first habit, in file order, that the gate admits
/// and whose effects expand.
fn intent_of(
    canon: &Canon,
    world: &ecs::World,
    presence: &Presence,
    actor_id: &ActorId,
) -> Option<Intent> {
    if !is_eligible(canon, world, presence, actor_id) {
        return None;
    }
    let habits = canon.rules.habits_of(actor_id);
    habits.into_iter().find_map(|rule| {
        let intent = Intent::habit(world, actor_id, &rule.local_id);
        let context = Context::new(canon, world, presence, &intent);
        let runs =
            gate::admit(&context, rule, None).is_ok() && expand::preview(&context, rule).is_ok();
        runs.then_some(intent)
    })
}

/// This NPC's habits whose `if` holds, for its view.
pub(crate) fn list_whose_if_holds(
    canon: &Canon,
    world: &ecs::World,
    presence: &Presence,
    npc_id: &ActorId,
) -> Vec<String> {
    let habits = canon.rules.habits_of(npc_id).into_iter();
    habits
        .filter(|rule| {
            let (verb, conds) = (&rule.local_id, &rule.if_);
            cond::if_holds(canon, world, presence, npc_id, verb, Target::Empty, conds)
        })
        .map(|rule| rule.local_id.clone())
        .collect()
}

/// Whether the engine may run one of this NPC's habits on its own this round: its idle
/// profile allows it, it is free, no seated player is with it, and its act window has room.
fn is_eligible(canon: &Canon, world: &ecs::World, presence: &Presence, npc_id: &ActorId) -> bool {
    if world.actors().get::<component::ControlType>(npc_id).is_pc() {
        return false;
    }
    let Some(profile) = canon.idle_profile(npc_id) else {
        return false;
    };
    if !profile.eligible {
        return false;
    }
    let act = world.actors().get::<component::Act>(npc_id);
    let act_state = act.get();
    let Some(place) = world.actors().get::<component::Place>(npc_id).place_id() else {
        return false;
    };
    if act.is_busy(world.world_time) || is_player_at(world, presence, place.as_str()) {
        return false;
    }
    if act_state.last_act_round == world.round_index {
        return false;
    }
    if presence.players == 0 {
        u64::from(act_state.idle_acts_this_window) < canon.budget.idle_acts_per_window
    } else {
        u64::from(act_state.live_acts_this_window) < canon.budget.live_acts_per_window
    }
}

/// A seated player's PC is in `place`.
pub(crate) fn is_player_at(world: &ecs::World, presence: &Presence, place: &str) -> bool {
    world.actors_at(place).iter().any(|id| {
        let is_pc = world.actors().get::<component::ControlType>(id).is_pc();
        presence.actors_present.contains(id) && is_pc
    })
}
