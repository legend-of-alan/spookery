//! Admit an intent, or refuse it with a reason.

use serde::{Deserialize, Serialize};

use crate::canon::Canon;
use crate::intent::{Intent, Source, Target, Viewer};
use crate::model::{Presence, State};
use crate::rule::{Owner, Rule, TargetKind, When};
use crate::rules::cond;
use crate::rules::cost;
use crate::rules::expr::Ctx;
use crate::rules::habits::is_eligible;
use crate::rules::perceive::perceive_as;
use crate::Reason;

/// Returned from the gate or a stage to the host table, which turns it into a
/// `protocol::Refusal`, and stored as written on the round line's `refused`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Refusal {
    pub id: String,
    pub code: Reason,
    /// The cost shortfall, or the sentence a locked exit shows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl Refusal {
    /// `intent` refused with `code` and no detail.
    pub(crate) fn new(intent: &Intent, code: Reason) -> Self {
        Self {
            id: intent.id.clone(),
            code,
            detail: None,
        }
    }
}

/// A verb word, or this actor's habit local id. A full id does not select a rule.
pub fn rule_for<'a>(canon: &'a Canon, now: &State, intent: &Intent) -> Result<&'a Rule, Reason> {
    let habit = canon
        .rules
        .habits_of(intent.actor.actor_id())
        .into_iter()
        .find(|rule| rule.local_id == intent.move_name);
    let verb = verb_rule(canon, now, intent);
    match intent.source {
        Source::Engine => habit.or(verb).ok_or(Reason::UnknownMove),
        Source::Player | Source::System if matches!(intent.actor, Viewer::Npc(_)) => {
            habit.or(verb).ok_or(Reason::UnknownMove)
        }
        Source::Player | Source::System => {
            if verb.is_none() && habit.is_some() {
                return Err(Reason::NotInSpec);
            }
            verb.ok_or(Reason::NotInSpec)
        }
    }
}

fn verb_rule<'a>(canon: &'a Canon, now: &State, intent: &Intent) -> Option<&'a Rule> {
    let hint = match &intent.target {
        Target::Actor(token) | Target::Item(token) => Some(token.as_str()),
        _ => None,
    };
    let actor = now.actors.get(intent.actor.actor_id());
    let place = actor.map(|actor| actor.place());
    let wielded = actor.and_then(|actor| actor.wielded());
    verb_for(canon, place, &intent.move_name, hint, wielded)
}

/// A wielded weapon's `attack` rule, then the owned rule for `hint`, then the place, then the shared verb.
pub fn verb_for<'a>(
    canon: &'a Canon,
    place: Option<&str>,
    verb: &str,
    hint: Option<&str>,
    wielded: Option<&str>,
) -> Option<&'a Rule> {
    if verb == "attack" {
        if let Some(item) = wielded.filter(|item| !item.is_empty()) {
            if let Some(rule) = owner_verb(canon, &Owner::Item(item.to_string()), verb) {
                return Some(rule);
            }
        }
    }
    if let Some(token) = hint.filter(|token| !token.is_empty()) {
        if let Some(rule) = owner_verb(canon, &Owner::Item(token.to_string()), verb) {
            return Some(rule);
        }
        if let Some(rule) = owner_verb(canon, &Owner::Cast(token.to_string()), verb) {
            return Some(rule);
        }
    }
    if let Some(place) = place {
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

/// Pick the rule for an intent, then `admit` it.
pub fn gate<'a>(
    canon: &'a Canon,
    now: &State,
    presence: &Presence,
    intent: &Intent,
) -> Result<&'a Rule, Refusal> {
    let rule = rule_for(canon, now, intent).map_err(|code| Refusal::new(intent, code))?;
    admit(canon, now, presence, intent, rule).map(|()| rule)
}

/// Check a rule already picked. A player-sourced habit skips its `if`. Cost is checked for every rule.
pub fn admit(
    canon: &Canon,
    now: &State,
    presence: &Presence,
    intent: &Intent,
    rule: &Rule,
) -> Result<(), Refusal> {
    if let Some(actor) = now.actors.get(intent.actor.actor_id()) {
        if actor.is_busy(now.world_time) && matches!(rule.when, When::Verb { .. }) {
            return Err(Refusal::new(intent, Reason::Busy));
        }
    }
    crate::travel::admit(canon, now, presence, intent, rule)?;
    if intent.source == Source::Player && !can_name(canon, now, presence, intent) {
        return Err(Refusal::new(intent, Reason::NotInSpec));
    }
    if let When::Verb { target, .. } = &rule.when {
        let shaped = match target {
            TargetKind::None => matches!(intent.target, Target::Empty),
            TargetKind::Place => matches!(intent.target, Target::Place(_)),
            TargetKind::Exit => matches!(intent.target, Target::Exit(_)),
            TargetKind::Actor => matches!(intent.target, Target::Actor(_)),
            TargetKind::Item => matches!(intent.target, Target::Item(_)),
        };
        if !shaped {
            return Err(Refusal::new(intent, Reason::Schema));
        }
    }
    if intent.source == Source::Engine && matches!(intent.actor, Viewer::Player(_)) {
        return Err(Refusal::new(intent, Reason::EngineOnPlayer));
    }
    if intent.source == Source::Engine
        && matches!(intent.actor, Viewer::Npc(_))
        && !is_eligible(canon, now, presence, intent.actor.actor_id())
    {
        return Err(Refusal::new(intent, Reason::Ineligible));
    }
    let skip_if = intent.source == Source::Player && matches!(rule.when, When::Habit);
    if !skip_if {
        cond::holds(canon, now, &Ctx { intent, presence }, &rule.if_)
            .map_err(|code| Refusal::new(intent, code))?;
    }
    if let Some(cost) = &rule.cost {
        cost::afford(canon, now, presence, intent, cost).map_err(|missing| Refusal {
            id: intent.id.clone(),
            code: Reason::CantAfford,
            detail: Some(missing),
        })?;
    }
    Ok(())
}

/// A player names only what they hold or what `look` shows them.
fn can_name(canon: &Canon, now: &State, presence: &Presence, intent: &Intent) -> bool {
    let id = match &intent.target {
        Target::Empty => return true,
        Target::Actor(id)
        | Target::Item(id)
        | Target::Place(id)
        | Target::Exit(id)
        | Target::Clock(id) => id.as_str(),
    };
    if now
        .actors
        .get(intent.actor.actor_id())
        .is_some_and(|actor| actor.item_count(id) > 0)
    {
        return true;
    }
    let Ok(view) = perceive_as(canon, now, presence, &intent.actor) else {
        return false;
    };
    view.place == id
        || view.visible_actors.iter().any(|other| other == id)
        || view.items.iter().any(|item| item.id == id)
        || view.exits.iter().any(|exit| exit.name == id)
        || view.clocks.iter().any(|clock| clock.id == id)
}
