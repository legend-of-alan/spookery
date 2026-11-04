//! Pick the rule a move runs, then admit it or refuse it with a reason.

use serde::{Deserialize, Serialize};

use crate::canon::Canon;
use crate::intent::{Intent, Source, Target, Viewer};
use crate::model::{Presence, State};
use crate::rules::cond;
use crate::rules::context::Context;
use crate::rules::cost;
use crate::rules::habits::is_eligible;
use crate::rules::perceive::perceive_as;
use crate::rules::rule::{Owner, Rule, TargetKind, When};
use crate::Reason;

/// One refused intent: its id, its code, and a detail when there is one. Returned from the
/// gate or a stage to the host table, which turns it into a `protocol::Refusal`, and stored as
/// written on the round line's `refused`.
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

/// The rule `actor`'s `verb` runs, the queue's and the round's alike. An NPC's habit is
/// picked by its local id first; any other verb is `verb_for`'s, looking in `hint` first.
/// A full rule id picks nothing. A player's move never runs a habit, and a missing rule is
/// `NOT_IN_SPEC` for it and `UNKNOWN_MOVE` for anyone else.
pub(crate) fn pick<'a>(
    canon: &'a Canon,
    state: &State,
    actor: &Viewer,
    source: Source,
    verb: &str,
    hint: Option<&str>,
) -> Result<&'a Rule, Reason> {
    let found = verb_for(canon, state, actor.actor_id(), verb, hint);
    if source == Source::Player && matches!(actor, Viewer::Player(_)) {
        return found.ok_or(Reason::NotInSpec);
    }
    let habits = canon.rules.habits_of(actor.actor_id());
    let habit = habits.into_iter().find(|rule| rule.local_id == verb);
    habit.or(found).ok_or(Reason::UnknownMove)
}

/// The owner an intent's target names for `pick` to look in first: an actor or an item.
pub(crate) fn hint_of(target: &Target) -> Option<&str> {
    match target {
        Target::Actor(token) | Target::Item(token) => Some(token),
        _ => None,
    }
}

/// The wielded item's rule, then the rule of the item or cast member `hint` names, then the
/// place's own, then the shared verb. Only `when: verb` rules are read, never a habit. A
/// cast member's rule is reached only by naming them, and the gate takes it only while they
/// are in sight (`can_name`). A place's rule is picked only there, so its `here` is that
/// place.
pub(crate) fn verb_for<'a>(
    canon: &'a Canon,
    state: &State,
    actor_id: &str,
    verb: &str,
    hint: Option<&str>,
) -> Option<&'a Rule> {
    let actor = state.actors.get(actor_id);
    let wielded = actor.and_then(|actor| actor.wielded.as_deref());
    if let Some(item) = wielded.filter(|item| !item.is_empty()) {
        if let Some(rule) = owner_verb(canon, &Owner::Item(item.to_string()), verb) {
            return Some(rule);
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
    if let Some(place) = actor.map(|actor| &actor.place) {
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

/// Pick the intent's rule, then `admit` it.
pub(crate) fn gate<'a>(context: &Context<'a>) -> Result<&'a Rule, Refusal> {
    let intent = context.intent;
    let (actor, verb, hint) = (&intent.actor, &intent.move_name, hint_of(&intent.target));
    let picked = pick(
        context.canon,
        context.state,
        actor,
        intent.source,
        verb,
        hint,
    );
    let rule = picked.map_err(|code| Refusal::new(intent, code))?;
    admit(context, rule).map(|()| rule)
}

/// Check a rule already picked. A player-sourced habit skips its `if`. Cost is checked for every rule.
pub(crate) fn admit(context: &Context, rule: &Rule) -> Result<(), Refusal> {
    let (state, intent) = (context.state, context.intent);
    if let Some(actor) = state.actors.get(intent.actor.actor_id()) {
        if actor.is_busy(state.world_time) && matches!(rule.when, When::Verb { .. }) {
            return Err(Refusal::new(intent, Reason::Busy));
        }
    }
    crate::rules::travel::admit(context, rule)?;
    if intent.source == Source::Player && !can_name(context) {
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
    if intent.source == Source::Engine
        && matches!(intent.actor, Viewer::Npc(_))
        && !is_eligible(
            context.canon,
            state,
            context.presence,
            intent.actor.actor_id(),
        )
    {
        return Err(Refusal::new(intent, Reason::Ineligible));
    }
    let skip_if = intent.source == Source::Player && matches!(rule.when, When::Habit);
    if !skip_if {
        cond::all_hold(context, &rule.if_).map_err(|code| Refusal::new(intent, code))?;
    }
    if let Some(cost) = &rule.cost {
        cost::afford(context, cost)?;
    }
    Ok(())
}

/// A seat names only what its actor holds or what `look` shows them.
fn can_name(context: &Context) -> bool {
    let intent = context.intent;
    let Some(id) = intent.target.token() else {
        return true;
    };
    let names = nameable(
        context.canon,
        context.state,
        context.presence,
        &intent.actor,
    );
    names.iter().any(|name| name == id)
}

/// Every id a seat may name: the items its actor holds, then the place, other actors, items
/// and exits `look` shows them. A seat never names its own actor.
pub(crate) fn nameable(
    canon: &Canon,
    state: &State,
    presence: &Presence,
    viewer: &Viewer,
) -> Vec<String> {
    let mut names: Vec<String> = match state.actors.get(viewer.actor_id()) {
        Some(actor) => actor
            .inventory
            .keys()
            .filter(|id| actor.item_count(id) > 0)
            .cloned()
            .collect(),
        None => Vec::new(),
    };
    if let Ok(view) = perceive_as(canon, state, presence, viewer) {
        names.push(view.place);
        let me = viewer.actor_id();
        names.extend(view.visible_actors.into_iter().filter(|id| id != me));
        names.extend(view.items.into_iter().map(|item| item.id));
        names.extend(view.exits.into_iter().map(|exit| exit.name));
    }
    names
}
