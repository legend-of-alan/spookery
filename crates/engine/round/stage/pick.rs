//! Take queued intents and habits, in the order this round will try them.

use std::collections::BTreeSet;

use crate::canon::Canon;
use crate::intent::{Intent, Target, Viewer};
use crate::rules::context::Context;
use crate::rules::effect::EffectKind;
use crate::rules::expand;
use crate::rules::gate;
use crate::rules::habits::{self, is_player_at};
use crate::rules::rule::{TargetKind, When};
use crate::rules::travel::find_exit;
use crate::state::{Presence, State};
use crate::Reason;

use crate::round::dice::shuffle_key;
use crate::round::log::RoundLog;
use crate::round::stage::apply::refuse;

/// The queue and the habits, stamped, in priority order, cut to the budget. Each intent past
/// the cut is refused `BUDGET`.
pub(crate) fn run(
    canon: &Canon,
    state: &State,
    presence: &Presence,
    queue: &[Intent],
    log: &mut RoundLog,
) -> Result<Vec<Intent>, Reason> {
    let mut candidates = queue.to_vec();
    candidates.extend(habits::intents(canon, state, presence));
    for intent in &mut candidates {
        stamp_target(canon, state, presence, intent);
    }
    candidates.sort_by_cached_key(|intent| priority(canon, state, presence, intent));
    check_unique_ids(&candidates)?;
    let limit = (canon.budget.max_intents_applied as usize).min(candidates.len());
    for intent in candidates.split_off(limit) {
        let refusal = gate::Refusal::new(&intent, Reason::Budget);
        refuse(log, &intent, refusal, false);
    }
    Ok(candidates)
}

/// Fill an empty target from a preview of the intent's effects, a place it moves to, and
/// its note recipients, so priority sees what it touches. A verb
/// with no target keeps it empty: a filled one would make the gate refuse `SCHEMA`.
fn stamp_target(canon: &Canon, state: &State, presence: &Presence, intent: &mut Intent) {
    let (verb, hint) = (&intent.move_name, gate::hint_of(&intent.target));
    let Some(actor) = &intent.actor else {
        return;
    };
    let Ok(rule) = gate::pick(canon, state, actor, intent.source, verb, hint) else {
        return;
    };
    let context = Context::new(canon, state, presence, intent);
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
            if let EffectKind::Location { to, .. } = &effect.kind {
                intent.target = Target::Place(to.clone().into());
            }
        }
        if let EffectKind::Note { to, .. } = &effect.kind {
            intent.args.message_to = Some(to.clone());
        }
    }
}

/// Refuse `SCHEMA` when a GM-chosen id equals a habit's: the one way two intents share an id.
fn check_unique_ids(candidates: &[Intent]) -> Result<(), Reason> {
    let mut seen = BTreeSet::new();
    for intent in candidates {
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

fn priority(canon: &Canon, state: &State, presence: &Presence, intent: &Intent) -> Priority {
    // Player lines that share a priority keep the queue order. NPC ties still shuffle.
    let (actor, shuffle) = match &intent.actor {
        Some(Viewer::Player(_)) => (ActorRank::Player, [0u8; 32]),
        Some(Viewer::Npc(id)) => (
            ActorRank::Npc,
            shuffle_key(&state.world_id, state.round_index, id),
        ),
        None => (ActorRank::Npc, [0u8; 32]),
    };
    let claim = if touches_seated_player(canon, state, presence, intent) {
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
    state: &State,
    presence: &Presence,
    intent: &Intent,
) -> bool {
    let present = &presence.actors_present;
    let target_hit = match &intent.target {
        Target::Actor(id) => present.contains(id),
        Target::Item(id) => present.iter().any(|actor| *actor == **id),
        Target::Place(place) => is_player_at(state, presence, place.as_str()),
        Target::Exit(name) => intent
            .actor_id()
            .and_then(|id| state.actors().stores().place.get(id))
            .and_then(|place| find_exit(canon, place, name))
            .is_some_and(|exit| is_player_at(state, presence, &exit.to)),
        _ => false,
    };
    let message_hit = intent
        .args
        .message_to
        .as_ref()
        .is_some_and(|ids| ids.iter().any(|id| present.iter().any(|actor| actor == id)));
    target_hit || message_hit
}
