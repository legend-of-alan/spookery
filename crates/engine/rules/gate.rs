//! Pick the rule a move runs, then admit it or refuse it with a reason.

use serde::{Deserialize, Serialize};
use spookery_protocol::is_named;

use crate::canon::Canon;
use crate::intent::{Intent, Source, Target, Viewer};
use crate::rules::cond;
use crate::rules::context::Context;
use crate::rules::cost;
use crate::rules::perceive::perceive_as;
use crate::rules::rule::{Owner, Rule, TargetKind, When};
use crate::state::id::ActorId;
use crate::state::nested_map;
use crate::state::{Presence, State};
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
        Target::Actor(id) => Some(id.as_str()),
        Target::Item(token) => Some(token),
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
    actor_id: &ActorId,
    verb: &str,
    hint: Option<&str>,
) -> Option<&'a Rule> {
    let wielded = state.actors().wielded.get(actor_id).map(String::as_str);
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
    if let Some(place) = state.actors().place.get(actor_id) {
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
pub(crate) fn rule_of<'c>(context: &Context<'c, '_>) -> Result<&'c Rule, Refusal> {
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
/// A verb's target is one the actor may name, read from `nameable` when the caller listed
/// it already. The `if` and the start cost are checked for every rule.
pub(crate) fn admit(
    context: &Context,
    rule: &Rule,
    nameable: Option<&[NamedId]>,
) -> Result<(), Refusal> {
    let (state, intent) = (context.state, context.intent);
    if let Some(act) = intent.actor_id().and_then(|id| state.actors().acts.get(id)) {
        let acted = act.last_act_round == intent.round_index;
        if acted && rule.start.counts_as_act && matches!(rule.when, When::Habit) {
            return Err(Refusal::new(intent, Reason::Ineligible));
        }
        if act.is_busy(state.world_time) {
            return Err(Refusal::new(intent, Reason::Busy));
        }
    }
    crate::rules::travel::admit(context, rule)?;
    if let When::Verb { target, .. } = &rule.when {
        if !can_name(context, nameable) {
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

/// The player command that drops the actor's own action under way.
pub(crate) const CANCEL: &str = "cancel";

/// Whether this intent is a player's `cancel`.
pub(crate) fn is_cancel(intent: &Intent) -> bool {
    intent.source == Source::Player && intent.move_name == CANCEL
}

/// A player may cancel only their own action under way, off any exit, whose rule is
/// `cancellable`. Anything else is `INELIGIBLE`.
pub(crate) fn admit_cancel(context: &Context) -> Result<(), Refusal> {
    let intent = context.intent;
    let actors = &context.state.actors();
    let actor_id = intent.actor_id();
    let pending = actor_id
        .and_then(|id| actors.acts.get(id))
        .and_then(|act| act.pending.as_ref());
    let walking = actor_id
        .and_then(|id| actors.place.get(id))
        .is_some_and(|place| crate::state::exit_parts(place).is_some());
    let rule = pending.and_then(|pending| context.canon.rules.get(&pending.rule));
    if walking || !rule.is_some_and(|rule| rule.cancellable) {
        return Err(Refusal::new(intent, Reason::Ineligible));
    }
    Ok(())
}

/// A seat names only what its actor holds or what `look` shows them: one of `nameable`,
/// listed here when the caller has not.
fn can_name(context: &Context, nameable: Option<&[NamedId]>) -> bool {
    let intent = context.intent;
    let Some(id) = intent.target.token() else {
        return true;
    };
    let Some(actor) = &intent.actor else {
        return false;
    };
    let listed;
    let nameable = match nameable {
        Some(nameable) => nameable,
        None => {
            listed = list_nameable(context.canon, context.state, context.presence, actor);
            &listed
        }
    };
    nameable.iter().any(|thing| thing.id == id)
}

/// An id a seat may name, with the names a player reads and types for it.
pub(crate) struct NamedId {
    pub(crate) id: String,
    /// The name the seat reads for it.
    pub(crate) name: String,
    /// The other names it answers to when typed: where an exit goes, by id and by name.
    pub(crate) aliases: Vec<String>,
}

impl NamedId {
    pub(crate) fn new(id: &str, name: String) -> Self {
        let id = id.to_string();
        Self {
            id,
            name,
            aliases: Vec::new(),
        }
    }
}

/// Everything a seat may name: the items its actor holds, then the place, other actors,
/// items and exits `look` shows them, each with the name that view gives it. A seat never
/// names its own actor.
pub(crate) fn list_nameable(
    canon: &Canon,
    state: &State,
    presence: &Presence,
    viewer: &Viewer,
) -> Vec<NamedId> {
    let mut nameable: Vec<NamedId> =
        nested_map::entries_of(&state.actors().items, viewer.actor_id())
            .filter(|(_, count)| **count > 0)
            .map(|(id, _)| NamedId::new(id, canon.display_name(id)))
            .collect();
    if let Ok(view) = perceive_as(canon, state, presence, viewer) {
        nameable.push(NamedId::new(&view.place, view.place_name));
        let me = viewer.actor_id();
        let actors = view.visible_actors.iter().zip(view.actor_names);
        let others = actors.filter(|(id, _)| *id != me);
        nameable.extend(others.map(|(id, name)| NamedId::new(id.as_str(), name)));
        let items = view.items.into_iter();
        nameable.extend(items.map(|item| NamedId::new(&item.id, item.name)));
        nameable.extend(view.exits.into_iter().map(|exit| NamedId {
            name: exit.display_name().to_string(),
            aliases: vec![exit.to, exit.place_name],
            id: exit.name,
        }));
    }
    nameable
}

/// The id `typed` names among `nameable`, by the id or any name, ignoring case and outer
/// spaces. `None` when nothing has that name. When two or more ids have it, the `Err` is the
/// detail of the `AMBIGUOUS_NAME` refusal: the name and those ids, `Briar: briar, oden`.
pub(crate) fn find_named<'a>(
    nameable: &'a [NamedId],
    typed: &str,
) -> Result<Option<&'a str>, String> {
    let has_name = |thing: &NamedId| {
        let mut names = [&thing.id, &thing.name].into_iter().chain(&thing.aliases);
        names.any(|name| is_named(typed, name))
    };
    let mut ids: Vec<&str> = (nameable.iter().filter(|thing| has_name(thing)))
        .map(|thing| thing.id.as_str())
        .collect();
    ids.sort_unstable();
    ids.dedup();
    match ids[..] {
        [] => Ok(None),
        [id] => Ok(Some(id)),
        _ => Err(format!("{}: {}", typed.trim(), ids.join(", "))),
    }
}
