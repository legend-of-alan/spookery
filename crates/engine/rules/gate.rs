//! Pick the rule a move runs, then admit it or refuse it with a reason.

use serde::{Deserialize, Serialize};
use spookery_protocol::is_named;

use crate::canon::Canon;
use crate::ecs::id::ActorId;
use crate::ecs::{self, component, Presence};
use crate::intent::{Intent, Source, Target, Viewer};
use crate::rules::cond;
use crate::rules::context::Context;
use crate::rules::cost;
use crate::rules::perceive;
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

/// The rule `actor`'s `verb_id` runs, the queue's and the round's alike. An NPC's habit is
/// picked by its local id first; any other verb is `verb_for`'s, looking in `hint` first.
/// A full rule id picks nothing. A player's move never runs a habit, and a missing rule is
/// `NOT_IN_SPEC` for it and `UNKNOWN_MOVE` for anyone else.
pub(crate) fn pick<'a>(
    canon: &'a Canon,
    world: &ecs::World,
    actor: &Viewer,
    source: Source,
    verb_id: &str,
    hint: Option<&str>,
) -> Result<&'a Rule, Reason> {
    let found = verb_for(canon, world, actor.actor_id(), verb_id, hint);
    if source == Source::Player && matches!(actor, Viewer::Player(_)) {
        return found.ok_or(Reason::NotInSpec);
    }
    let habits = canon.rules.habits_of(actor.actor_id());
    let habit = habits.into_iter().find(|rule| rule.local_id == verb_id);
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
    world: &ecs::World,
    actor_id: &ActorId,
    verb_id: &str,
    hint: Option<&str>,
) -> Option<&'a Rule> {
    let wielded = world.actors().get::<component::WieldedItem>(actor_id).get();
    if let Some(item) = wielded.filter(|item| !item.is_empty()) {
        if let Some(rule) = owner_verb(canon, &Owner::Item(item.to_string()), verb_id) {
            return Some(rule);
        }
    }
    if let Some(token) = hint.filter(|token| !token.is_empty()) {
        if let Some(rule) = owner_verb(canon, &Owner::Item(token.to_string()), verb_id) {
            return Some(rule);
        }
        if let Some(rule) = owner_verb(canon, &Owner::Cast(token.to_string()), verb_id) {
            return Some(rule);
        }
    }
    if let Some(place) = world.actors().get::<component::Place>(actor_id).get() {
        if let Some(rule) = owner_verb(canon, &Owner::Place(place.to_string()), verb_id) {
            return Some(rule);
        }
    }
    owner_verb(canon, &Owner::Verb, verb_id)
}

fn owner_verb<'a>(canon: &'a Canon, owner: &Owner, verb_id: &str) -> Option<&'a Rule> {
    canon.rules.iter().find(|rule| {
        &rule.owner == owner
            && matches!(&rule.when, When::Verb { verb_id: id, .. } if id == verb_id)
    })
}

/// The rule the intent's verb picks, for its actor and target.
pub(crate) fn rule_of<'c>(context: &Context<'c, '_>) -> Result<&'c Rule, Refusal> {
    let intent = context.intent;
    let (verb_id, hint) = (&intent.move_id, hint_of(&intent.target));
    let actor = intent
        .actor
        .as_ref()
        .ok_or_else(|| Refusal::new(intent, Reason::UnknownId))?;
    let picked = pick(
        context.canon,
        context.world,
        actor,
        intent.source,
        verb_id,
        hint,
    );
    picked.map_err(|code| Refusal::new(intent, code))
}

/// Check a rule already picked, whoever queued it. A habit whose start counts as an act is
/// `INELIGIBLE` once its NPC has acted this round, and any rule is `BUSY` while its actor is.
/// A verb's target is one the actor may name, read from `ids` when the caller listed
/// it already. The `if` and the start cost are checked for every rule.
pub(crate) fn admit(
    context: &Context,
    rule: &Rule,
    ids: Option<&[NamedId]>,
) -> Result<(), Refusal> {
    let (world, intent) = (context.world, context.intent);
    if let Some(id) = intent.actor_id() {
        let act = world.actors().get::<component::Act>(id);
        let acted = act
            .get()
            .is_some_and(|act| act.last_act_round == intent.round_index);
        if acted && rule.start.counts_as_act && matches!(rule.when, When::Habit) {
            return Err(Refusal::new(intent, Reason::Ineligible));
        }
        if act.is_busy(world.world_time) {
            return Err(Refusal::new(intent, Reason::Busy));
        }
    }
    crate::rules::travel::admit(context, rule)?;
    if let When::Verb { target, .. } = &rule.when {
        if !can_name(context, ids) {
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
    intent.source == Source::Player && intent.move_id == CANCEL
}

/// A player may cancel only their own action under way, off any exit, whose rule is
/// `cancellable`. Anything else is `INELIGIBLE`.
pub(crate) fn admit_cancel(context: &Context) -> Result<(), Refusal> {
    let intent = context.intent;
    let actors = context.world.actors();
    let actor_id = intent.actor_id();
    let pending = actor_id
        .and_then(|id| actors.get::<component::Act>(id).get())
        .and_then(|act| act.pending.as_ref());
    let walking = actor_id
        .and_then(|id| actors.get::<component::Place>(id).get())
        .is_some_and(|place| crate::ecs::exit_parts(place).is_some());
    let rule = pending.and_then(|pending| context.canon.rules.get(&pending.rule));
    if walking || !rule.is_some_and(|rule| rule.cancellable) {
        return Err(Refusal::new(intent, Reason::Ineligible));
    }
    Ok(())
}

/// A seat names only what its actor holds or what `look` shows them: one of `ids`,
/// listed here when the caller has not.
fn can_name(context: &Context, ids: Option<&[NamedId]>) -> bool {
    let intent = context.intent;
    let Some(id) = intent.target.token() else {
        return true;
    };
    let Some(actor) = &intent.actor else {
        return false;
    };
    let listed;
    let ids = match ids {
        Some(ids) => ids,
        None => {
            listed = list_ids(context.canon, context.world, context.presence, actor);
            &listed
        }
    };
    ids.iter().any(|named_id| named_id.id == id)
}

/// An id a seat may name, with the names a player reads and types for it.
pub(crate) struct NamedId {
    pub(crate) id: String,
    /// The name the seat reads for it.
    pub(crate) name: String,
    /// The other words it answers to when typed: where an exit goes, by place id and name.
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
pub(crate) fn list_ids(
    canon: &Canon,
    world: &ecs::World,
    presence: &Presence,
    viewer: &Viewer,
) -> Vec<NamedId> {
    let Ok(view) = perceive::build_view(canon, world, presence, viewer) else {
        return Vec::new();
    };
    let named_items = |items: Vec<perceive::ItemStack>| {
        (items.into_iter()).map(|item| NamedId::new(&item.id, item.name))
    };
    let mut ids: Vec<NamedId> = named_items(view.items_carried).collect();
    ids.push(NamedId::new(&view.place, view.place_name));
    let me = viewer.actor_id();
    let actors = view.visible_actors.iter().zip(view.actor_names);
    let others = actors.filter(|(id, _)| *id != me);
    ids.extend(others.map(|(id, name)| NamedId::new(id.as_str(), name)));
    ids.extend(named_items(view.items));
    ids.extend(view.exits.into_iter().map(|exit| NamedId {
        id: exit.exit_id,
        name: exit.name,
        aliases: vec![exit.place_id, exit.place_name],
    }));
    ids
}

/// The id `text` names among `ids`, by the id or any name, ignoring case and outer
/// spaces. `None` when nothing has that name. When two or more ids have it, the `Err` is the
/// detail of the `AMBIGUOUS_NAME` refusal: the text and those ids, `Briar: bram, briar`.
pub(crate) fn find_id<'a>(ids: &'a [NamedId], text: &str) -> Result<Option<&'a str>, String> {
    let has_name = |named_id: &NamedId| {
        let mut candidates = [&named_id.id, &named_id.name]
            .into_iter()
            .chain(&named_id.aliases);
        candidates.any(|candidate| is_named(text, candidate))
    };
    let mut matches: Vec<&str> = (ids.iter().filter(|named_id| has_name(named_id)))
        .map(|named_id| named_id.id.as_str())
        .collect();
    matches.sort_unstable();
    matches.dedup();
    match matches[..] {
        [] => Ok(None),
        [id] => Ok(Some(id)),
        _ => Err(format!("{}: {}", text.trim(), matches.join(", "))),
    }
}
