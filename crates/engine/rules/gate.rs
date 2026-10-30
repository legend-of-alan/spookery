//! Admit an intent, or refuse it with a reason.

use crate::canon::Canon;
use crate::error::Reason;
use crate::intent::{Intent, Source, Target, Viewer};
use crate::model::{Presence, State};
use crate::rule::{Owner, Rule, TargetKind, When};
use crate::rules::cond;
use crate::rules::cost;
use crate::rules::expr::Ctx;
use crate::rules::habits::is_eligible;
use crate::rules::perceive::perceive_as;

/// A gate refusal. `Reason` stays fieldless. A cost shortfall keeps the missing name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GateRefusal {
    Code(Reason),
    CantAfford(String),
    /// A refusal whose player text is not the token. `code` is still the token.
    Text {
        code: Reason,
        text: String,
    },
}

impl GateRefusal {
    pub fn reason(&self) -> Reason {
        match self {
            GateRefusal::Code(reason) | GateRefusal::Text { code: reason, .. } => *reason,
            GateRefusal::CantAfford(_) => Reason::CantAfford,
        }
    }

    pub fn token(&self) -> &'static str {
        self.reason().as_str()
    }

    /// The shortfall, or the sentence a locked exit shows.
    pub fn missing(&self) -> Option<&str> {
        match self {
            GateRefusal::CantAfford(what) | GateRefusal::Text { text: what, .. } => {
                Some(what.as_str())
            }
            GateRefusal::Code(_) => None,
        }
    }
}

impl From<Reason> for GateRefusal {
    fn from(reason: Reason) -> Self {
        GateRefusal::Code(reason)
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
) -> Result<&'a Rule, GateRefusal> {
    let rule = rule_for(canon, now, intent)?;
    admit(canon, now, presence, intent, rule).map(|()| rule)
}

/// Check a rule already picked. A player-sourced habit skips its `if`. Cost is checked for every rule.
pub fn admit(
    canon: &Canon,
    now: &State,
    presence: &Presence,
    intent: &Intent,
    rule: &Rule,
) -> Result<(), GateRefusal> {
    if let Some(actor) = now.actors.get(intent.actor.actor_id()) {
        if actor.is_busy(now.world_time) && matches!(rule.when, When::Verb { .. }) {
            return Err(Reason::Busy.into());
        }
    }
    crate::travel::admit(canon, now, presence, intent, rule)?;
    if intent.source == Source::Player && !can_name(canon, now, presence, intent) {
        return Err(Reason::NotInSpec.into());
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
            return Err(Reason::Schema.into());
        }
    }
    if intent.source == Source::Engine && matches!(intent.actor, Viewer::Player(_)) {
        return Err(Reason::EngineOnPlayer.into());
    }
    if intent.source == Source::Engine
        && matches!(intent.actor, Viewer::Npc(_))
        && !is_eligible(canon, now, presence, intent.actor.actor_id())
    {
        return Err(Reason::Ineligible.into());
    }
    let skip_if = intent.source == Source::Player && matches!(rule.when, When::Habit);
    if !skip_if {
        cond::holds(canon, now, &Ctx { intent, presence }, &rule.if_)?;
    }
    if let Some(cost) = &rule.cost {
        if let Err(shortfall) = cost::afford(canon, now, presence, intent, cost) {
            return Err(GateRefusal::CantAfford(shortfall.missing));
        }
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
