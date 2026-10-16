use crate::canon::Canon;
use crate::error::Reason;
use crate::intent::{ActorKind, Intent, Source, Target};
use crate::model::Now;
use crate::rules::cond;
use crate::rules::cost;
use crate::rules::expr::Ctx;
use crate::rules::habits::is_eligible;
use crate::rules::rule::{Owner, Rule, TargetKind, When};

/// A gate refusal. `Reason` stays fieldless. A cost gap keeps the missing name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Miss {
    Code(Reason),
    CantAfford(String),
    /// A refusal whose player text is not the token. `code` is still the token.
    Text {
        code: Reason,
        text: String,
    },
}

impl Miss {
    pub fn reason(&self) -> Reason {
        match self {
            Miss::Code(reason) | Miss::Text { code: reason, .. } => *reason,
            Miss::CantAfford(_) => Reason::CantAfford,
        }
    }

    pub fn token(&self) -> &'static str {
        self.reason().token()
    }

    /// Journal word. A gap is `CANT_AFFORD` plus the meter or item id.
    pub fn named(&self) -> String {
        match self {
            Miss::CantAfford(what) => format!("CANT_AFFORD {what}"),
            Miss::Code(reason) | Miss::Text { code: reason, .. } => {
                reason.jsonl_reason().to_string()
            }
        }
    }

    /// The gap, or the sentence a locked exit shows.
    pub fn missing(&self) -> Option<&str> {
        match self {
            Miss::CantAfford(what) | Miss::Text { text: what, .. } => Some(what.as_str()),
            Miss::Code(_) => None,
        }
    }
}

impl From<Reason> for Miss {
    fn from(reason: Reason) -> Self {
        Miss::Code(reason)
    }
}

/// A verb word, or this actor's habit local id. A full id does not select a rule.
pub fn resolve<'a>(canon: &'a Canon, now: &Now, intent: &Intent) -> Result<&'a Rule, Reason> {
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
    let hint = match &intent.target {
        Target::Actor(token) => Some(token.as_str()),
        _ => None,
    };
    let actor = now.actors.get(&intent.actor_id);
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

/// Gate every intent. A player-sourced habit skips its `if`. Cost is checked for every rule.
pub fn gate<'a>(canon: &'a Canon, now: &Now, intent: &Intent) -> Result<&'a Rule, Miss> {
    let rule = resolve(canon, now, intent)?;
    if let Some(actor) = now.actors.get(&intent.actor_id) {
        if actor.is_busy(now.world_time) && matches!(rule.when, When::Verb { .. }) {
            return Err(Reason::Busy.into());
        }
    }
    crate::rules::exit::admit(canon, now, intent)?;
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
                return Err(Reason::UnknownKey.into());
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
                return Err(Reason::UnknownKey.into());
            }
        }
        Target::Clock(id) => {
            if !now.clocks.contains_key(id) {
                return Err(Reason::UnknownKey.into());
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
            return Err(Reason::Schema.into());
        }
    }
    if intent.source == Source::Kernel && intent.actor_kind == ActorKind::Player {
        return Err(Reason::KernelOnPlayer.into());
    }
    if intent.source == Source::Kernel
        && intent.actor_kind == ActorKind::Npc
        && !is_eligible(canon, now, &intent.actor_id)
    {
        return Err(Reason::Ineligible.into());
    }
    let skip_if = intent.source == Source::Player && matches!(rule.when, When::Habit);
    if !skip_if {
        cond::holds(canon, now, &Ctx { intent }, &rule.if_)?;
    }
    if let Some(cost) = &rule.cost {
        if let Err(gap) = cost::can_pay(canon, now, &intent.actor_id, cost) {
            return Err(Miss::CantAfford(gap.what));
        }
    }
    Ok(rule)
}
