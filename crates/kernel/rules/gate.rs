use crate::canon::Canon;
use crate::error::Reason;
use crate::intent::{ActorKind, Intent, Source, Target};
use crate::model::Now;
use crate::rules::cond;
use crate::rules::cost;
use crate::rules::expr::Ctx;
use crate::rules::habits::is_eligible;
use crate::rules::rule::{Owner, Rule, TargetKind, When};

/// The one verb rule for this intent, or this actor's habit of the same local id.
pub fn resolve<'a>(canon: &'a Canon, now: &Now, intent: &Intent) -> Result<&'a Rule, Reason> {
    if let Some(rule) = canon.rules.get(&intent.move_name) {
        return Ok(rule);
    }
    let habit = canon
        .rules
        .habits_of(&intent.actor_id)
        .into_iter()
        .find(|rule| rule.local_id == intent.move_name);
    let verb = verb_rule(canon, now, intent);
    match intent.source {
        Source::Kernel => habit.or(verb).ok_or(Reason::UnknownMove),
        Source::Player | Source::System if intent.actor_kind == ActorKind::Npc => {
            if let Some(rule) = habit {
                return Ok(rule);
            }
            verb.ok_or(Reason::UnknownMove)
        }
        Source::Player | Source::System => {
            if verb.is_none() && habit.is_some() {
                return Err(Reason::NotInSpec);
            }
            verb.ok_or(Reason::NotInSpec)
        }
    }
}

fn verb_rule<'a>(canon: &'a Canon, now: &Now, intent: &Intent) -> Option<&'a Rule> {
    let verb = &intent.move_name;
    if let Target::Actor(token) = &intent.target {
        if let Some(rule) = owner_verb(canon, &Owner::Item(token.clone()), verb) {
            return Some(rule);
        }
        if let Some(rule) = owner_verb(canon, &Owner::Cast(token.clone()), verb) {
            return Some(rule);
        }
    }
    if let Some(place) = now.actors.get(&intent.actor_id).map(|actor| actor.place()) {
        if let Some(rule) = owner_verb(canon, &Owner::Place(place.to_string()), verb) {
            return Some(rule);
        }
    }
    canon.rules.verb(verb)
}

fn owner_verb<'a>(canon: &'a Canon, owner: &Owner, verb: &str) -> Option<&'a Rule> {
    canon.rules.iter().find(|rule| {
        &rule.owner == owner && matches!(&rule.when, When::Verb { verb: name, .. } if name == verb)
    })
}

/// Gate every intent. A player-sourced habit skips its `if`. Cost is checked for every rule.
pub fn gate<'a>(canon: &'a Canon, now: &Now, intent: &Intent) -> Result<&'a Rule, Reason> {
    let rule = resolve(canon, now, intent)?;
    match &intent.target {
        Target::Empty => {}
        Target::Place(id) => {
            if !matches!(
                rule.when,
                When::Verb {
                    target: TargetKind::Place | TargetKind::Exit,
                    ..
                }
            ) && !canon.places.contains_key(id)
                && !now.places.contains_key(id)
            {
                return Err(Reason::UnknownKey);
            }
        }
        Target::Actor(id) => {
            let item = matches!(
                rule.when,
                When::Verb {
                    target: TargetKind::Item,
                    ..
                }
            );
            if !item && !now.actors.contains_key(id) {
                return Err(Reason::UnknownKey);
            }
        }
        Target::Clock(id) => {
            if !now.clocks.contains_key(id) {
                return Err(Reason::UnknownKey);
            }
        }
    }
    if let When::Verb { target, .. } = &rule.when {
        let shaped = match target {
            TargetKind::None => matches!(intent.target, Target::Empty),
            TargetKind::Place | TargetKind::Exit => matches!(intent.target, Target::Place(_)),
            TargetKind::Actor | TargetKind::Item => matches!(intent.target, Target::Actor(_)),
        };
        if !shaped {
            return Err(Reason::Schema);
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
    let skip_if = intent.source == Source::Player && matches!(rule.when, When::Habit);
    if !skip_if {
        cond::holds(canon, now, &Ctx { intent }, &rule.if_)?;
    }
    if let Some(cost) = &rule.cost {
        if let Err(gap) = cost::can_pay(canon, now, &intent.actor_id, cost) {
            let _ = gap.what;
            return Err(Reason::CantAfford);
        }
    }
    Ok(rule)
}
