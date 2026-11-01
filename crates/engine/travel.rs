//! Exits, admission, and the journeys that follow them.

use std::collections::BTreeSet;

use crate::canon::{Canon, Exit, PlaceDef};
use crate::draft::Draft;
use crate::intent::{Intent, Source, Target, Viewer};
use crate::key::StateKey;
use crate::model::{ActorLive, PendingAction, PendingKind, Presence, State};
use crate::model::{MINUTES_PER_DAY, MINUTES_PER_HOUR};
use crate::rule::{Cond, EffectTemplate, LocationTo, Rule, TargetKind, When};
use crate::rules::case::Case;
use crate::rules::cond;
use crate::rules::cost;
use crate::rules::expr::Dice;
use crate::rules::gate;
use crate::visibility::can_see_fact;
use crate::Reason;
pub use spookery_protocol::{exit_phrase, ExitView};

pub(crate) fn find_exit<'a>(canon: &'a Canon, from: &str, name: &str) -> Option<&'a Exit> {
    canon
        .places
        .get(from)
        .and_then(|place| place.exits.iter().find(|exit| exit.name == name))
}

/// The exit this rule travels by: the one its first `location via` that is not a teleport
/// names. `None` when the rule does not travel.
pub(crate) fn exit_of<'a>(case: &Case<'a>, rule: &Rule) -> Result<Option<&'a Exit>, gate::Refusal> {
    let via = rule.top_templates().find_map(|effect| match effect {
        EffectTemplate::Location {
            to: LocationTo::Via(via),
            teleport: false,
        } => Some(via),
        _ => None,
    });
    match via {
        Some(via) => exit_via(case, via).map_err(|code| gate::Refusal::new(case.intent, code)),
        None => Ok(None),
    }
}

/// The exit `via` names from the actor's place, or `NO_EXIT`. `via: target` is the intent's
/// exit target, and `None` when the target is not an exit; any other word is an exit name.
pub(crate) fn exit_via<'a>(case: &Case<'a>, via: &str) -> Result<Option<&'a Exit>, Reason> {
    let name = match (via, &case.intent.target) {
        ("target", Target::Exit(name)) => name.as_str(),
        ("target", _) => return Ok(None),
        (name, _) => name,
    };
    let actor = case.now.actors.get(case.intent.actor.actor_id());
    let from = actor.map(ActorLive::place).unwrap_or("");
    find_exit(case.canon, from, name)
        .map(Some)
        .ok_or(Reason::NoExit)
}

/// Refuse a missing, hidden, locked, or unpaid exit. A clear exit is `Ok`.
pub(crate) fn admit(case: &Case, rule: &Rule) -> Result<(), gate::Refusal> {
    let Some(exit) = exit_of(case, rule)? else {
        return Ok(());
    };
    let (canon, now, intent) = (case.canon, case.now, case.intent);
    if !is_revealed(canon, now, intent.actor.actor_id(), exit) {
        return Err(gate::Refusal::new(intent, Reason::NoExit));
    }
    if is_locked(canon, now, case.presence, intent.actor.actor_id(), exit) {
        let text = if exit.locked_text.is_empty() {
            "The way is locked.".to_string()
        } else {
            exit.locked_text.clone()
        };
        return Err(gate::Refusal {
            id: intent.id.clone(),
            code: Reason::NoExit,
            detail: Some(text),
        });
    }
    cost::afford(case, &exit.cost)
}

fn is_revealed(canon: &Canon, now: &State, actor_id: &str, exit: &Exit) -> bool {
    if !exit.hidden {
        return true;
    }
    let Some(fact) = &exit.revealed_by else {
        return false;
    };
    let Some(viewer) = now.viewer_of(actor_id) else {
        return false;
    };
    can_see_fact(canon, now, &viewer, fact)
}

fn is_locked(canon: &Canon, now: &State, presence: &Presence, actor_id: &str, exit: &Exit) -> bool {
    if exit.if_.is_empty() {
        return false;
    }
    let Some(viewer) = now.viewer_of(actor_id) else {
        return true;
    };
    let target = Target::Exit(exit.name.clone());
    !if_holds(canon, now, presence, &viewer, "travel", target, &exit.if_)
}

/// Whether `conds` hold for `viewer` doing `verb` to `target`. `help` and exit locks read an
/// `if` this way: they have no intent, so they read it against a probe with no id.
pub(crate) fn if_holds(
    canon: &Canon,
    now: &State,
    presence: &Presence,
    viewer: &Viewer,
    verb: &str,
    target: Target,
    conds: &[Cond],
) -> bool {
    let probe = Intent::new(
        String::new(),
        now.round_index,
        viewer.clone(),
        Source::Player,
        verb,
        target,
    );
    cond::holds(&Case::new(canon, now, presence, &probe), conds).is_ok()
}

/// The exits of `place` this actor is allowed to see. A hidden exit stays off the list.
pub(crate) fn exits_at(
    canon: &Canon,
    now: &State,
    presence: &Presence,
    actor_id: &str,
    place: &PlaceDef,
) -> Vec<ExitView> {
    place
        .exits
        .iter()
        .filter(|exit| is_revealed(canon, now, actor_id, exit))
        .map(|exit| ExitView {
            name: exit.name.clone(),
            to: exit.to.clone(),
            place_name: place_name(canon, &exit.to),
            minutes: exit.cost.minutes,
            locked: is_locked(canon, now, presence, actor_id, exit),
        })
        .collect()
}

/// The exits of this actor's place that they are allowed to see.
pub(crate) fn visible_exits(
    canon: &Canon,
    now: &State,
    presence: &Presence,
    actor_id: &str,
) -> Vec<ExitView> {
    let actor = now.actors.get(actor_id);
    match actor.and_then(|actor| canon.places.get(actor.place())) {
        Some(place) => exits_at(canon, now, presence, actor_id, place),
        None => Vec::new(),
    }
}

/// World minutes until the rule and its exit finish. Zero means it finishes now.
pub(crate) fn span(case: &Case, rule: &Rule) -> u64 {
    let mut minutes = rule.cost.as_ref().map(|cost| cost.minutes).unwrap_or(0);
    if let Ok(Some(exit)) = exit_of(case, rule) {
        minutes = minutes.saturating_add(exit.cost.minutes);
    }
    minutes
}

/// One journey whose minute has been reached. The actor is no longer busy.
pub(crate) struct DueAction {
    pub actor_id: String,
    pub until: u64,
    pub pending: PendingAction,
}

/// Clear every journey that finishes at or before `end`, earliest first.
pub(crate) fn take_due_actions(draft: &mut Draft, end: u64) -> Result<Vec<DueAction>, Reason> {
    let mut due_keys: Vec<(u64, String)> = draft
        .state()
        .actors
        .iter()
        .filter_map(|(id, actor)| {
            let act = actor.act();
            let until = act.busy_until?;
            if act.pending.is_none() || until > end {
                return None;
            }
            Some((until, id.clone()))
        })
        .collect();
    due_keys.sort();
    let mut due_actions = Vec::new();
    for (until, actor_id) in due_keys {
        draft.track(StateKey::ActorBusy(actor_id.clone()))?;
        let pending = {
            let actor = draft
                .state_mut()
                .actors
                .get_mut(&actor_id)
                .ok_or(Reason::UnknownId)?;
            let act = actor.act_mut();
            act.busy_until = None;
            act.pending.take()
        };
        let Some(pending) = pending else {
            continue;
        };
        due_actions.push(DueAction {
            actor_id,
            until,
            pending,
        });
    }
    Ok(due_actions)
}

/// Pay the exit, then remember the finish minute. The move itself waits.
pub(crate) fn begin_action(
    draft: &mut Draft,
    intent: &Intent,
    rule: &Rule,
    world_time_before: u64,
    minutes: u64,
    dice: &mut dyn Dice,
) -> Result<(), Reason> {
    let case = Case::new(draft.canon(), draft.state(), draft.presence, intent);
    let kind = match exit_of(&case, rule).ok().flatten().cloned() {
        Some(exit) => {
            cost::pay(draft, intent, &exit.cost, dice)?;
            PendingKind::Arrive {
                place: exit.to,
                via: exit.name,
            }
        }
        None => PendingKind::Finish {
            target: intent.target.clone(),
            rule: rule.id.clone(),
        },
    };
    let until = world_time_before.saturating_add(minutes);
    let pending = PendingAction {
        verb: intent.move_name.clone(),
        source: intent.source,
        kind,
    };
    draft.track(StateKey::ActorBusy(intent.actor.actor_id().to_string()))?;
    let act = draft
        .state_mut()
        .actors
        .get_mut(intent.actor.actor_id())
        .ok_or(Reason::UnknownId)?
        .act_mut();
    act.busy_until = Some(until);
    act.pending = Some(pending);
    Ok(())
}

pub(crate) fn map_lines(canon: &Canon, now: &State, presence: &Presence, actor_id: &str) -> String {
    let Some(actor) = now.actors.get(actor_id) else {
        return String::new();
    };
    let mut lines = Vec::new();
    for place_id in actor.visited() {
        let Some(place) = canon.places.get(place_id) else {
            continue;
        };
        lines.push(place_name(canon, place_id));
        for view in exits_at(canon, now, presence, actor_id, place) {
            let phrase = exit_phrase(&view.name, view.minutes, view.locked);
            lines.push(format!("  {phrase}"));
        }
    }
    lines.join("\n")
}

/// Verbs whose `if` holds for this actor, then the visible exits, then a busy line.
pub(crate) fn help_lines(
    canon: &Canon,
    now: &State,
    presence: &Presence,
    actor_id: &str,
) -> String {
    let mut lines = allowed_verbs(canon, now, presence, actor_id);
    lines.extend(exit_phrases(canon, now, presence, actor_id));
    if let Some(status) = busy_line(now, actor_id) {
        lines.push(status);
    }
    lines.join("\n")
}

/// The place name, the prose this actor may read, and the visible exits.
pub(crate) fn look_lines(
    canon: &Canon,
    now: &State,
    presence: &Presence,
    actor_id: &str,
) -> String {
    let mut head = Vec::new();
    if let Some(actor) = now.actors.get(actor_id) {
        let name = place_name(canon, actor.place());
        if !name.is_empty() {
            head.push(name);
        }
    }
    let prose = place_prose(canon, now, actor_id);
    if !prose.is_empty() {
        head.push(prose);
    }
    let exits = exit_phrases(canon, now, presence, actor_id).join("\n");
    let head = head.join("\n");
    if head.is_empty() {
        exits
    } else {
        format!("{head}\n\n{exits}")
    }
}

/// `on the stairs until 21:40` while a journey is still running.
pub(crate) fn busy_line(now: &State, actor_id: &str) -> Option<String> {
    let actor = now.actors.get(actor_id)?;
    if !actor.is_busy(now.world_time) {
        return None;
    }
    let until = actor.act().busy_until?;
    let via = match actor.act().pending.as_ref().map(|pending| &pending.kind) {
        Some(PendingKind::Arrive { via, .. }) if !via.is_empty() => via,
        _ => return None,
    };
    Some(format!("on the {via} until {}", time_of_day(until)))
}

/// Place prose for this actor's current place.
pub(crate) fn place_prose(canon: &Canon, now: &State, actor_id: &str) -> String {
    let Some(actor) = now.actors.get(actor_id) else {
        return String::new();
    };
    let Some(place) = canon.places.get(actor.place()) else {
        return String::new();
    };
    place
        .body
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Minutes from midnight as `21:40`.
fn time_of_day(minutes: u64) -> String {
    let hour = (minutes % MINUTES_PER_DAY) / MINUTES_PER_HOUR;
    let minute = minutes % MINUTES_PER_HOUR;
    format!("{hour:02}:{minute:02}")
}

fn exit_phrases(canon: &Canon, now: &State, presence: &Presence, actor_id: &str) -> Vec<String> {
    let views = visible_exits(canon, now, presence, actor_id);
    if views.is_empty() {
        return vec!["Exits:".to_string(), "none".to_string()];
    }
    let mut lines = vec!["Exits:".to_string()];
    for view in views {
        lines.push(exit_phrase(&view.name, view.minutes, view.locked));
    }
    lines
}

/// A verb is listed when its `if` holds for one of the targets it could name. Busy, cost and
/// locks are not read.
fn allowed_verbs(canon: &Canon, now: &State, presence: &Presence, actor_id: &str) -> Vec<String> {
    let Some(actor) = now.actors.get(actor_id) else {
        return Vec::new();
    };
    let (viewer, place, wielded) = (actor.viewer(actor_id), Some(actor.place()), actor.wielded());
    let mut seen = BTreeSet::new();
    let mut names = Vec::new();
    for rule in canon.rules.player_verbs() {
        let When::Verb { verb, target } = &rule.when else {
            continue;
        };
        if !seen.insert(verb.clone()) {
            continue;
        }
        let hints = target_candidates(canon, now, presence, actor_id, *target);
        let allowed = hints.into_iter().any(|hint| {
            let Some(rule) = gate::verb_for(canon, place, verb, hint.as_deref(), wielded) else {
                return false;
            };
            let When::Verb { target: chosen, .. } = &rule.when else {
                return false;
            };
            let target = Target::new(*chosen, &hint.unwrap_or_default());
            if_holds(canon, now, presence, &viewer, verb, target, &rule.if_)
        });
        if allowed {
            names.push(verb.clone());
        }
    }
    names
}

fn target_candidates(
    canon: &Canon,
    now: &State,
    presence: &Presence,
    actor_id: &str,
    target: TargetKind,
) -> Vec<Option<String>> {
    let hints = match target {
        TargetKind::None => Vec::new(),
        TargetKind::Exit => visible_exits(canon, now, presence, actor_id)
            .into_iter()
            .map(|exit| exit.name)
            .collect(),
        TargetKind::Place => canon.places.keys().cloned().collect(),
        TargetKind::Item => now
            .actors
            .get(actor_id)
            .map(|actor| actor.inventory().keys().cloned().collect())
            .unwrap_or_default(),
        TargetKind::Actor => match now.actors.get(actor_id) {
            Some(actor) => now.actors_at(actor.place()),
            None => Vec::new(),
        },
    };
    if hints.is_empty() {
        vec![None]
    } else {
        hints.into_iter().map(Some).collect()
    }
}

pub(crate) fn place_name(canon: &Canon, id: &str) -> String {
    canon
        .places
        .get(id)
        .map_or_else(|| id.to_string(), |place| place.display_name.clone())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::*;

    #[test]
    fn help_and_look_name_the_shop() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packs/oneshot-docks");
        let canon = crate::canon::load_canon(&root).unwrap();
        let now = crate::store::codec::parse_save(
            &fs::read_to_string(root.join("state/save.yaml")).unwrap(),
        )
        .unwrap();
        let help = help_lines(&canon, &now, &Presence::default(), "pc_rowan");
        assert!(help.lines().any(|line| line == "wait"), "{help}");
        assert!(help.lines().any(|line| line == "travel"), "{help}");
        assert!(!help.lines().any(|line| line == "use"), "{help}");
        assert!(help.contains("stairs (locked) — 2 min"), "{help}");
        assert!(help.contains("docks_street — 0 min"), "{help}");
        assert!(!help.contains("→"), "{help}");
        let look = look_lines(&canon, &now, &Presence::default(), "pc_rowan");
        assert!(look.contains("Docks Shop"), "{look}");
        assert!(
            look.contains("The shop faces the water. Tins sit in a row."),
            "{look}"
        );
        assert!(look.contains("stairs (locked) — 2 min"), "{look}");
    }
}
