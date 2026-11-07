//! Pick the rule a move runs, then admit it or refuse it with a reason.

use serde::{Deserialize, Serialize};

use crate::canon::Canon;
use crate::intent::{Intent, Source, Target, Viewer};
use crate::model::{Presence, State};
use crate::rules::cond;
use crate::rules::context::Context;
use crate::rules::cost;
use crate::rules::effect::Holder;
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

/// The rule the intent's verb picks, for its actor and target.
pub(crate) fn rule_of<'a>(context: &Context<'a>) -> Result<&'a Rule, Refusal> {
    let intent = context.intent;
    let (verb, hint) = (&intent.move_name, hint_of(&intent.target));
    let actor = intent
        .actor
        .as_ref()
        .ok_or_else(|| Refusal::new(intent, Reason::UnknownId))?;
    let picked = pick(
        context.canon,
        context.state,
        actor,
        intent.source,
        verb,
        hint,
    );
    picked.map_err(|code| Refusal::new(intent, code))
}

/// Check a rule already picked, whoever queued it. A habit whose start counts as an act is
/// `INELIGIBLE` once its NPC has acted this round, and any rule is `BUSY` while its actor is.
/// A verb's target is one the actor may name. The `if` and the start cost are checked for
/// every rule.
pub(crate) fn admit(context: &Context, rule: &Rule) -> Result<(), Refusal> {
    let (state, intent) = (context.state, context.intent);
    if let Some(actor) = intent.actor_id().and_then(|id| state.actors.get(id)) {
        let acted = actor.act.last_act_round == intent.round_index;
        if acted && rule.start.counts_as_act && matches!(rule.when, When::Habit) {
            return Err(Refusal::new(intent, Reason::Ineligible));
        }
        if actor.is_busy(state.world_time) {
            return Err(Refusal::new(intent, Reason::Busy));
        }
    }
    crate::rules::travel::admit(context, rule)?;
    if let When::Verb { target, .. } = &rule.when {
        if !can_name(context) {
            return Err(Refusal::new(intent, Reason::NotInSpec));
        }
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
    cond::all_hold(context, &rule.if_).map_err(|code| Refusal::new(intent, code))?;
    if let Some(cost) = &rule.start.cost {
        cost::afford(context, cost)?;
    }
    Ok(())
}

/// The player word that drops the actor's own action under way.
pub(crate) const CANCEL: &str = "cancel";

/// Whether this intent is a player's `cancel`.
pub(crate) fn is_cancel(intent: &Intent) -> bool {
    intent.source == Source::Player && intent.move_name == CANCEL
}

/// A player may cancel only their own action under way, off any exit, whose rule is
/// `cancellable`. Anything else is `INELIGIBLE`.
pub(crate) fn admit_cancel(context: &Context) -> Result<(), Refusal> {
    let intent = context.intent;
    let actor = intent
        .actor_id()
        .and_then(|id| context.state.actors.get(id));
    let pending = actor.and_then(|actor| actor.act.pending.as_ref());
    let walking = actor.is_some_and(|actor| crate::model::exit_parts(&actor.place).is_some());
    let rule = pending.and_then(|pending| context.canon.rules.get(&pending.rule));
    if walking || !rule.is_some_and(|rule| rule.cancellable) {
        return Err(Refusal::new(intent, Reason::Ineligible));
    }
    Ok(())
}

/// A seat names only what its actor holds or what `look` shows them.
fn can_name(context: &Context) -> bool {
    let intent = context.intent;
    let Some(id) = intent.target.token() else {
        return true;
    };
    let Some(actor) = &intent.actor else {
        return false;
    };
    let names = nameable(context.canon, context.state, context.presence, actor);
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
    let me = Holder::Actor(viewer.actor_id().to_string());
    let mut names: Vec<String> = state.items.on(&me).map(|(id, _)| id.clone()).collect();
    if let Ok(view) = perceive_as(canon, state, presence, viewer) {
        names.push(view.place);
        let me = viewer.actor_id();
        names.extend(view.visible_actors.into_iter().filter(|id| id != me));
        names.extend(view.items.into_iter().map(|item| item.id));
        names.extend(view.exits.into_iter().map(|exit| exit.name));
    }
    names
}
