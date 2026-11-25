//! Take queued intents and habits, in the order this round will try them, and pick each one's
//! rule. The gate runs at queue and start; the check rolls when the move lands.

use std::collections::BTreeSet;

use crate::canon::Canon;
use crate::ecs::id::ActorId;
use crate::ecs::{self, component, Presence};
use crate::intent::{Intent, Target};
use crate::rules::context::Context;
use crate::rules::effect::EffectKind;
use crate::rules::expand;
use crate::rules::gate;
use crate::rules::habits::{self, is_player_at};
use crate::rules::rule::{Rule, TargetKind, When};
use crate::Reason;

use crate::round::dice::shuffle_key;
use crate::round::stage::apply::refuse;
use crate::round::Ledger;

/// One candidate and how pick left it for apply.
pub(crate) struct Candidate<'c> {
    pub intent: Intent,
    pub resolution: Resolution<'c>,
}

/// How pick left an intent for apply.
pub(crate) enum Resolution<'c> {
    /// Picked against the state. Apply admits `rule` again against the working copy when it starts,
    /// then pays for and expands it, so a move pays the cost of the rule it picked.
    Ready { rule: &'c Rule },
    /// A player's `cancel`, which runs no rule.
    Cancel,
    /// No rule picked: journaled, and told to a player.
    Refused(gate::Refusal),
}

/// The queue and the habits, stamped, in priority order, cut to the budget, each with its rule.
/// Each intent past the cut is refused `BUDGET`.
pub(crate) fn run<'c>(
    canon: &'c Canon,
    world: &ecs::World,
    presence: &Presence,
    queue: &[Intent],
    ledger: &mut Ledger,
) -> Result<Vec<Candidate<'c>>, Reason> {
    let mut intents = queue.to_vec();
    intents.extend(habits::intents(canon, world, presence));
    for intent in &mut intents {
        stamp_target(canon, world, presence, intent);
    }
    intents.sort_by_cached_key(|intent| priority(canon, world, presence, intent));
    check_unique_ids(&intents)?;
    let limit = (canon.budget.max_intents_applied as usize).min(intents.len());
    for intent in intents.split_off(limit) {
        let refusal = gate::Refusal::new(&intent, Reason::Budget);
        refuse(world, ledger, &intent, refusal);
    }
    Ok(intents
        .into_iter()
        .map(|intent| Candidate {
            resolution: resolve(canon, world, presence, &intent),
            intent,
        })
        .collect())
}

fn resolve<'c>(
    canon: &'c Canon,
    world: &ecs::World,
    presence: &Presence,
    intent: &Intent,
) -> Resolution<'c> {
    if gate::is_cancel(intent) {
        return Resolution::Cancel;
    }
    match gate::rule_of(&Context::new(canon, world, presence, intent)) {
        Ok(rule) => Resolution::Ready { rule },
        Err(refusal) => Resolution::Refused(refusal),
    }
}

/// Fill an empty target from a preview of the intent's effects, a place it moves to, and
/// its note recipients, so priority sees what it touches. A verb
/// with no target keeps it empty: a filled one would make the gate refuse `SCHEMA`.
fn stamp_target(canon: &Canon, world: &ecs::World, presence: &Presence, intent: &mut Intent) {
    let (verb, hint) = (&intent.move_id, gate::hint_of(&intent.target));
    let Some(actor_id) = &intent.actor_id else {
        return;
    };
    let Ok(rule) = gate::pick(canon, world, actor_id, intent.source, verb, hint) else {
        return;
    };
    let context = Context::new(canon, world, presence, intent);
    let Ok(effects) = expand::preview(&context, rule) else {
        return;
    };
    let keeps_empty = matches!(
        &rule.when,
        When::Verb {
            target: TargetKind::None,
            ..
        }
    );
    for effect in &effects {
        if !keeps_empty && intent.target == Target::Empty {
            if let EffectKind::Location {
                to: component::ActorLocation::Place(to),
                ..
            } = &effect.kind
            {
                intent.target = Target::Place(to.clone());
            }
        }
        if let EffectKind::Note { to_ids, .. } = &effect.kind {
            intent.args.message_to = Some(to_ids.clone());
        }
    }
}

/// Refuse `SCHEMA` when a GM-chosen id equals a habit's: the one way two intents share an id.
fn check_unique_ids(intents: &[Intent]) -> Result<(), Reason> {
    let mut seen = BTreeSet::new();
    for intent in intents {
        if !seen.insert(&intent.id) {
            return Err(Reason::Schema);
        }
    }
    Ok(())
}

/// Who acts first: players, then a claimed target, then the shuffle.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct Priority {
    actor: ActorRank,
    claim: SeatedRank,
    shuffle: [u8; 32],
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum ActorRank {
    Player,
    Npc,
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum SeatedRank {
    Claimed,
    Unclaimed,
}

/// Player lines that share a priority keep the queue order. NPC ties shuffle.
fn priority(canon: &Canon, world: &ecs::World, presence: &Presence, intent: &Intent) -> Priority {
    let is_pc = |id: &ActorId| world.actors().get::<component::ControlType>(id).is_pc();
    let (actor, shuffle) = match &intent.actor_id {
        Some(id) if is_pc(id) => (ActorRank::Player, [0u8; 32]),
        Some(id) => (
            ActorRank::Npc,
            shuffle_key(&world.seed, world.round_index, id),
        ),
        None => (ActorRank::Npc, [0u8; 32]),
    };
    let claim = if touches_seated_player(canon, world, presence, intent) {
        SeatedRank::Claimed
    } else {
        SeatedRank::Unclaimed
    };
    Priority {
        actor,
        claim,
        shuffle,
    }
}

fn touches_seated_player(
    canon: &Canon,
    world: &ecs::World,
    presence: &Presence,
    intent: &Intent,
) -> bool {
    let present = &presence.actors_present;
    let target_hit = match &intent.target {
        Target::Actor(id) => present.contains(id),
        Target::Item(id) => present.iter().any(|actor| *actor == **id),
        Target::Place(place) => is_player_at(world, presence, place.as_str()),
        Target::Exit(exit_id) => intent
            .actor_id
            .as_ref()
            .and_then(|id| world.actors().get::<component::Place>(id).place_id())
            .and_then(|place| canon.find_exit(place.as_str(), exit_id))
            .is_some_and(|exit| is_player_at(world, presence, exit.to_place_id.as_str())),
        _ => false,
    };
    let message_hit = intent
        .args
        .message_to
        .as_ref()
        .is_some_and(|ids| ids.iter().any(|id| present.iter().any(|actor| actor == id)));
    target_hit || message_hit
}
