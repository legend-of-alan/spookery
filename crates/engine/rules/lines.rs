//! The text of `help`, `look` and `map`.

use std::collections::BTreeSet;

use crate::canon::Canon;
use crate::intent::Target;
use crate::model::{PendingKind, Presence, State};
use crate::model::{MINUTES_PER_DAY, MINUTES_PER_HOUR};
use crate::rules::cond;
use crate::rules::gate;
use crate::rules::rule::{TargetKind, When};
use crate::rules::travel::{exits_at, visible_exits};
use spookery_protocol::exit_phrase;

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
    let viewer = actor.viewer(actor_id);
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
            let Some(rule) = gate::verb_for(canon, now, actor_id, verb, hint.as_deref()) else {
                return false;
            };
            let When::Verb { target: chosen, .. } = &rule.when else {
                return false;
            };
            let target = Target::new(*chosen, &hint.unwrap_or_default());
            cond::if_holds(canon, now, presence, &viewer, verb, target, &rule.if_)
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
