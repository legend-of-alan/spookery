use crate::canon::{Canon, Habit};
use crate::effect::{Effect, EffectClass};
use crate::eligibility::is_eligible;
use crate::error::Reason;
use crate::expand::expand_outcome;
use crate::ids::kernel_intent_id;
use crate::intent::{ActorKind, Intent, IntentArgs, Source, Target};
use crate::model::Now;
use crate::moves::Audience;

/// First habit that is on and whose row expands is the NPC's intent.
pub fn kernel_intents_for_eligible_npcs(canon: &Canon, now: &Now) -> Vec<Intent> {
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

pub fn allowed(canon: &Canon, now: &Now, intent: &Intent) -> Result<(), Reason> {
    if !canon.moves.contains_key(&intent.move_name) {
        return Err(Reason::UnknownMove);
    }
    match &intent.target {
        Target::Empty => {}
        Target::Place(id) => {
            if !canon.places.contains_key(id) && !now.places.contains_key(id) {
                return Err(Reason::UnknownKey);
            }
        }
        Target::Actor(id) => {
            if !now.actors.contains_key(id) {
                return Err(Reason::UnknownKey);
            }
        }
        Target::Clock(id) => {
            if !now.clocks.contains_key(id) {
                return Err(Reason::UnknownKey);
            }
        }
        Target::Rumor(id) => {
            if !now.rumors.iter().any(|rumor| rumor.id == *id) {
                return Err(Reason::UnknownKey);
            }
        }
    }
    if intent.source == Source::Kernel && intent.actor_kind == ActorKind::Player {
        return Err(Reason::KernelOnPlayer);
    }
    if intent.source == Source::Kernel
        && intent.actor_kind == ActorKind::Npc
        && !is_eligible(canon, now, &intent.actor_id)
    {
        return Err(Reason::Ineligible);
    }
    Ok(())
}

/// Habit ids that are on for this actor, in file order.
pub fn habits_on(canon: &Canon, now: &Now, actor_id: &str) -> Vec<String> {
    let Some(profile) = canon.profile(actor_id) else {
        return Vec::new();
    };
    let hour = world_hour(now.world_time);
    profile
        .habits
        .iter()
        .filter(|habit| habit_on(now, habit, hour))
        .map(|habit| habit.id.clone())
        .collect()
}

pub fn world_hour(world_time: u64) -> i64 {
    ((world_time / 60) % 24) as i64
}

fn select_one(canon: &Canon, now: &Now, actor_id: &str) -> Option<Intent> {
    let profile = canon.profile(actor_id)?;
    let hour = world_hour(now.world_time);
    for habit in &profile.habits {
        if !habit_on(now, habit, hour) {
            continue;
        }
        for row_name in &habit.rows {
            let row = canon.moves.get(row_name)?.row.as_ref()?;
            if row.audience != Some(Audience::Habit) {
                continue;
            }
            let mut intent = mint(now, actor_id, row_name);
            match expand_outcome(canon, now, &intent, None) {
                Ok(effects) => {
                    stamp(&mut intent, &effects);
                    return Some(intent);
                }
                Err(_) => continue,
            }
        }
    }
    None
}

fn habit_on(now: &Now, habit: &Habit, hour: i64) -> bool {
    if now.presence.humans == 0 && !habit.runs_while_empty {
        return false;
    }
    if let (Some(from), Some(to)) = (habit.from_hour, habit.to_hour) {
        if !hour_in(from, to, hour) {
            return false;
        }
    }
    if let (Some(clock_id), Some(at_least)) = (&habit.clock, habit.at_least) {
        let Some(clock) = now.clocks.get(clock_id) else {
            return false;
        };
        if clock.paused || clock.value < at_least {
            return false;
        }
    }
    if let Some(message_id) = &habit.unless_message {
        if now.messages.iter().any(|message| &message.id == message_id) {
            return false;
        }
    }
    true
}

fn hour_in(start: i64, end: i64, hour: i64) -> bool {
    if start < end {
        start <= hour && hour < end
    } else if start > end {
        hour >= start || hour < end
    } else {
        false
    }
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

/// Fill priority fields from the row. A target the row still has to read stays put.
pub fn stamp_for_priority(canon: &Canon, now: &Now, intent: &mut Intent) {
    if let Ok(effects) = expand_outcome(canon, now, intent, None) {
        stamp(intent, &effects);
    }
}

fn stamp(intent: &mut Intent, effects: &[Effect]) {
    for effect in effects {
        if intent.target == Target::Empty {
            match &effect.class {
                EffectClass::Location { to, .. } => {
                    intent.target = Target::Place(to.clone());
                }
                EffectClass::Rumor { rumor_id, .. } => {
                    intent.target = Target::Rumor(rumor_id.clone());
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

#[cfg(test)]
mod tests {
    use super::{hour_in, world_hour};

    #[test]
    fn hour_window_wraps_and_a_point_is_off() {
        assert_eq!(world_hour(1080), 18);
        assert_eq!(world_hour(0), 0);
        assert_eq!(world_hour(1440), 0);
        assert!(hour_in(8, 16, 8));
        assert!(hour_in(8, 16, 15));
        assert!(!hour_in(8, 16, 16));
        assert!(!hour_in(8, 16, 7));
        assert!(hour_in(18, 6, 18));
        assert!(hour_in(18, 6, 5));
        assert!(!hour_in(18, 6, 6));
        assert!(!hour_in(18, 6, 12));
        assert!(!hour_in(8, 8, 8));
    }
}
