//! Exits, admission, and the journeys that follow them.

use std::collections::BTreeSet;

use crate::canon::{Canon, Exit};
use crate::draft::Draft;
use crate::error::Reason;
use crate::intent::{Intent, Source, Target};
use crate::key::StateKey;
use crate::model::{PendingAction, PendingKind, Presence, State};
use crate::rule::{EffectTemplate, LocationTo, Rule, TargetKind, When};
use crate::rules::cond;
use crate::rules::cost;
use crate::rules::expr::{Ctx, Dice};
use crate::rules::gate::{self, GateRefusal};
use crate::visibility::can_see_fact;
pub use spookery_protocol::{exit_phrase, ExitView};

pub(crate) fn find_exit<'a>(canon: &'a Canon, from: &str, name: &str) -> Option<&'a Exit> {
    canon
        .places
        .get(from)
        .and_then(|place| place.exits.iter().find(|exit| exit.name == name))
}

/// The exit a `location via` effect names, if this rule travels.
pub(crate) fn exit_name(rule: &Rule, intent: &Intent) -> Option<String> {
    for effect in rule.top_templates() {
        if let EffectTemplate::Location {
            to: LocationTo::Via(via),
            teleport: false,
        } = effect
        {
            if via == "target" {
                if let Target::Place(name) | Target::Exit(name) = &intent.target {
                    return Some(name.clone());
                }
                return None;
            }
            return Some(via.clone());
        }
    }
    None
}

/// Refuse a missing, hidden, locked, or unpaid exit. A clear exit is `Ok`.
pub(crate) fn admit(
    canon: &Canon,
    now: &State,
    presence: &Presence,
    intent: &Intent,
    rule: &Rule,
) -> Result<(), GateRefusal> {
    let Some(token) = exit_name(rule, intent) else {
        return Ok(());
    };
    let from = now
        .actors
        .get(intent.actor.actor_id())
        .map(|actor| actor.place())
        .unwrap_or("");
    let Some(exit) = find_exit(canon, from, &token) else {
        return Err(Reason::NoExit.into());
    };
    if !is_revealed(canon, now, intent.actor.actor_id(), exit) {
        return Err(Reason::NoExit.into());
    }
    if is_locked(canon, now, presence, intent.actor.actor_id(), exit) {
        let text = if exit.locked_text.is_empty() {
            "The way is locked.".to_string()
        } else {
            exit.locked_text.clone()
        };
        return Err(GateRefusal::Text {
            code: Reason::NoExit,
            text,
        });
    }
    if let Err(shortfall) = cost::afford(canon, now, presence, intent, &exit.cost) {
        return Err(GateRefusal::CantAfford(shortfall.missing));
    }
    Ok(())
}

pub(crate) fn is_revealed(canon: &Canon, now: &State, actor_id: &str, exit: &Exit) -> bool {
    if !exit.hidden {
        return true;
    }
    let Some(fact) = &exit.revealed_by else {
        return false;
    };
    let Some(viewer) = now.actors.get(actor_id).map(|actor| actor.viewer(actor_id)) else {
        return false;
    };
    can_see_fact(canon, now, &viewer, fact)
}

pub(crate) fn is_locked(
    canon: &Canon,
    now: &State,
    presence: &Presence,
    actor_id: &str,
    exit: &Exit,
) -> bool {
    if exit.if_.is_empty() {
        return false;
    }
    let Some(actor) = now.actors.get(actor_id) else {
        return true;
    };
    let intent = Intent::new(
        String::new(),
        now.round_index,
        actor.viewer(actor_id),
        Source::Player,
        "travel",
        Target::Exit(exit.name.clone()),
    );
    cond::holds(
        canon,
        now,
        &Ctx {
            intent: &intent,
            presence,
        },
        &exit.if_,
    )
    .is_err()
}

/// Exits this actor is allowed to see. A hidden exit stays off the list.
pub fn visible_exits(
    canon: &Canon,
    now: &State,
    presence: &Presence,
    actor_id: &str,
) -> Vec<ExitView> {
    let Some(actor) = now.actors.get(actor_id) else {
        return Vec::new();
    };
    let Some(place) = canon.places.get(actor.place()) else {
        return Vec::new();
    };
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

/// World minutes until the rule and its exit finish. Zero means it finishes now.
pub fn span(canon: &Canon, now: &State, intent: &Intent, rule: &Rule) -> u64 {
    let mut minutes = rule.cost.as_ref().map(|cost| cost.minutes).unwrap_or(0);
    if let Some(exit) = used_exit(canon, now, intent, rule) {
        minutes = minutes.saturating_add(exit.cost.minutes);
    }
    minutes
}

pub(crate) fn used_exit<'a>(
    canon: &'a Canon,
    now: &State,
    intent: &Intent,
    rule: &Rule,
) -> Option<&'a Exit> {
    let token = exit_name(rule, intent)?;
    let from = now.actors.get(intent.actor.actor_id())?.place();
    find_exit(canon, from, &token)
}

/// One journey whose minute has been reached. The actor is no longer busy.
pub struct DueAction {
    pub actor_id: String,
    pub until: u64,
    pub pending: PendingAction,
}

/// Clear every journey that finishes at or before `end`, earliest first.
pub fn take_due_actions(draft: &mut Draft, end: u64) -> Result<Vec<DueAction>, Reason> {
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
pub fn begin_action(
    draft: &mut Draft,
    intent: &Intent,
    rule: &Rule,
    world_time_before: u64,
    minutes: u64,
    dice: &mut dyn Dice,
) -> Result<(), Reason> {
    let kind = match used_exit(draft.canon(), draft.state(), intent, rule).cloned() {
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

pub fn map_lines(canon: &Canon, now: &State, presence: &Presence, actor_id: &str) -> String {
    let Some(actor) = now.actors.get(actor_id) else {
        return String::new();
    };
    let mut lines = Vec::new();
    for place_id in actor.visited() {
        let Some(place) = canon.places.get(place_id) else {
            continue;
        };
        lines.push(place_name(canon, place_id));
        for exit in &place.exits {
            if !is_revealed(canon, now, actor_id, exit) {
                continue;
            }
            lines.push(format!(
                "  {}",
                exit_line(canon, now, presence, actor_id, exit)
            ));
        }
    }
    lines.join("\n")
}

/// Verbs whose `if` holds for this actor, then the visible exits, then a busy line.
pub fn help_lines(canon: &Canon, now: &State, presence: &Presence, actor_id: &str) -> String {
    let mut lines = allowed_verbs(canon, now, presence, actor_id);
    lines.extend(exit_phrases(canon, now, presence, actor_id));
    if let Some(status) = busy_line(now, actor_id) {
        lines.push(status);
    }
    lines.join("\n")
}

/// The place name, the prose this actor may read, and the visible exits.
pub fn look_lines(canon: &Canon, now: &State, presence: &Presence, actor_id: &str) -> String {
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
pub fn busy_line(now: &State, actor_id: &str) -> Option<String> {
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
pub fn place_prose(canon: &Canon, now: &State, actor_id: &str) -> String {
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
pub fn time_of_day(minutes: u64) -> String {
    let hour = (minutes / 60) % 24;
    let minute = minutes % 60;
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

fn allowed_verbs(canon: &Canon, now: &State, presence: &Presence, actor_id: &str) -> Vec<String> {
    if !now.actors.contains_key(actor_id) {
        return Vec::new();
    }
    let mut seen = BTreeSet::new();
    let mut names = Vec::new();
    for rule in canon.rules.player_verbs() {
        let When::Verb { verb, target } = &rule.when else {
            continue;
        };
        if !seen.insert(verb.clone()) {
            continue;
        }
        if is_verb_allowed(canon, now, presence, actor_id, verb, *target) {
            names.push(verb.clone());
        }
    }
    names
}

fn is_verb_allowed(
    canon: &Canon,
    now: &State,
    presence: &Presence,
    actor_id: &str,
    verb: &str,
    target: TargetKind,
) -> bool {
    let Some(actor) = now.actors.get(actor_id) else {
        return false;
    };
    let place = actor.place().to_string();
    let wielded = actor.wielded().map(str::to_string);
    let viewer = actor.viewer(actor_id);
    for hint in target_candidates(canon, now, presence, actor_id, target) {
        let Some(rule) = gate::verb_for(
            canon,
            Some(place.as_str()),
            verb,
            hint.as_deref(),
            wielded.as_deref(),
        ) else {
            continue;
        };
        if rule.if_.is_empty() {
            return true;
        }
        let When::Verb { target: chosen, .. } = &rule.when else {
            continue;
        };
        let token = hint.unwrap_or_default();
        let intent = Intent::new(
            String::new(),
            now.round_index,
            viewer.clone(),
            Source::Player,
            verb,
            Target::new(*chosen, &token),
        );
        if cond::holds(
            canon,
            now,
            &Ctx {
                intent: &intent,
                presence,
            },
            &rule.if_,
        )
        .is_ok()
        {
            return true;
        }
    }
    false
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
        TargetKind::Actor => {
            let here = now.actors.get(actor_id).map(|actor| actor.place());
            now.actors
                .iter()
                .filter(|(_, actor)| here.is_some_and(|place| actor.place() == place))
                .map(|(id, _)| id.clone())
                .collect()
        }
    };
    if hints.is_empty() {
        vec![None]
    } else {
        hints.into_iter().map(Some).collect()
    }
}

fn exit_line(
    canon: &Canon,
    now: &State,
    presence: &Presence,
    actor_id: &str,
    exit: &Exit,
) -> String {
    let locked = is_locked(canon, now, presence, actor_id, exit);
    exit_phrase(&exit.name, exit.cost.minutes, locked)
}

pub fn place_name(canon: &Canon, id: &str) -> String {
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
