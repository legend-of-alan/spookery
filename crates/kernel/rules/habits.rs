use crate::canon::Canon;
use crate::effect::{Effect, EffectClass};
use crate::expand::expand_outcome;
use crate::ids::kernel_intent_id;
use crate::intent::{ActorKind, Intent, IntentArgs, Source, Target};
use crate::model::Now;
use crate::rules::cond;
use crate::rules::expr::Ctx;
use crate::rules::gate;
use crate::rules::rule::When;

pub fn habit_intents(canon: &Canon, now: &Now) -> Vec<Intent> {
    let mut intents = Vec::new();
    for actor_id in now.actors.keys() {
        if !is_eligible(canon, now, actor_id) {
            continue;
        }
        if let Some(intent) = select_one(canon, now, actor_id) {
            intents.push(intent);
        }
    }
    intents
}

pub fn habits_on(canon: &Canon, now: &Now, actor_id: &str) -> Vec<String> {
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

pub fn stamp_for_priority(canon: &Canon, now: &Now, intent: &mut Intent) {
    if let Ok(effects) = expand_outcome(canon, now, intent, None) {
        stamp(intent, &effects);
    }
}

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

fn select_one(canon: &Canon, now: &Now, actor_id: &str) -> Option<Intent> {
    for rule in canon.rules.habits_of(actor_id) {
        if !matches!(rule.when, When::Habit) {
            continue;
        }
        let intent = mint(now, actor_id, &rule.local_id);
        if cond::holds(canon, now, &Ctx { intent: &intent }, &rule.if_).is_err() {
            continue;
        }
        if gate::gate(canon, now, &intent).is_err() {
            continue;
        }
        match expand_outcome(canon, now, &intent, None) {
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

fn mint(now: &Now, actor_id: &str, row: &str) -> Intent {
    Intent {
        id: kernel_intent_id(&now.shard_id, now.tick_index, actor_id, row),
        tick_index: now.tick_index,
        actor_id: actor_id.to_string(),
        actor_kind: ActorKind::Npc,
        source: Source::Kernel,
        move_name: row.to_string(),
        target: Target::Empty,
        args: IntentArgs::default(),
        rationale_code: String::new(),
    }
}

fn stamp(intent: &mut Intent, effects: &[Effect]) {
    for effect in effects {
        if intent.target == Target::Empty {
            match &effect.class {
                EffectClass::Location { to, .. } => {
                    intent.target = Target::Place(to.clone());
                }
                EffectClass::Clock { clock_id, .. } => {
                    intent.target = Target::Clock(clock_id.clone());
                }
                _ => {}
            }
        }
        if let EffectClass::Message { to, .. } = &effect.class {
            intent.args.message_to = Some(to.clone());
        }
    }
}
