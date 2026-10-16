//! Exits and the journeys that follow them.

use std::collections::BTreeSet;

use crate::canon::{Canon, Exit};
use crate::diagnostic::Diagnostic;
use crate::draft::Draft;
use crate::error::Reason;
use crate::firewall::allows;
use crate::intent::{ActorKind, Intent, IntentArgs, Source, Target, ViewerId};
use crate::key::StateKey;
use crate::model::{Now, Pending};
use crate::rules::cond;
use crate::rules::cost;
use crate::rules::exit;
use crate::rules::expr::Ctx;
use crate::rules::gate;
use crate::rules::rule::{Rule, TargetKind, When};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExitView {
    pub name: String,
    pub to: String,
    pub place_name: String,
    pub minutes: u64,
    pub locked: bool,
}

/// Exits this actor is allowed to see. A hidden exit stays off the list.
pub fn visible(canon: &Canon, now: &Now, actor_id: &str) -> Vec<ExitView> {
    let Some(actor) = now.actors.get(actor_id) else {
        return Vec::new();
    };
    let Some(place) = canon.places.get(actor.place()) else {
        return Vec::new();
    };
    place
        .exits
        .iter()
        .filter(|exit| exit::revealed(canon, now, actor_id, exit))
        .map(|exit| ExitView {
            name: exit.name.clone(),
            to: exit.to.clone(),
            place_name: place_name(canon, &exit.to),
            minutes: exit.minutes,
            locked: exit::locked(canon, now, actor_id, exit),
        })
        .collect()
}

/// World minutes until the rule and its exit finish. Zero means it finishes now.
pub fn span(canon: &Canon, now: &Now, intent: &Intent, rule: &Rule) -> u64 {
    let mut minutes = rule.cost.as_ref().map(|cost| cost.minutes).unwrap_or(0);
    if let Some(exit) = used_exit(canon, now, intent, rule) {
        minutes = minutes.saturating_add(exit.minutes);
    }
    minutes
}

pub(crate) fn used_exit<'a>(
    canon: &'a Canon,
    now: &Now,
    intent: &Intent,
    rule: &Rule,
) -> Option<&'a Exit> {
    let token = exit::token(rule, intent)?;
    let from = now.actors.get(&intent.actor_id)?.place();
    exit::named(canon, from, &token)
}

/// One journey whose minute has been reached. The actor is no longer busy.
pub struct Due {
    pub actor_id: String,
    pub until: u64,
    pub pending: Pending,
}

/// Clear every journey that finishes at or before `end`, earliest first.
pub fn take_due(draft: &mut Draft, end: u64) -> Result<Vec<Due>, Reason> {
    let mut due: Vec<(u64, String)> = draft
        .now()
        .actors
        .iter()
        .filter_map(|(id, actor)| {
            let act = actor.act();
            let until = act.busy_until?;
            if act.busy_for.is_none() || until > end {
                return None;
            }
            Some((until, id.clone()))
        })
        .collect();
    due.sort();
    let mut out = Vec::new();
    for (until, actor_id) in due {
        draft.track(StateKey::ActorBusy(actor_id.clone()))?;
        let pending = {
            let actor = draft
                .now_mut()
                .actors
                .get_mut(&actor_id)
                .ok_or(Reason::UnknownKey)?;
            let act = actor.act_mut();
            act.busy_until = None;
            act.busy_for.take()
        };
        let Some(pending) = pending else {
            continue;
        };
        out.push(Due {
            actor_id,
            until,
            pending,
        });
    }
    Ok(out)
}

/// Pay the exit, then remember the finish minute. The move itself waits.
pub fn begin(
    draft: &mut Draft,
    intent: &Intent,
    rule: &Rule,
    world_time_before: u64,
    minutes: u64,
) -> Result<(), Reason> {
    let travel = used_exit(draft.canon(), draft.now(), intent, rule)
        .map(|exit| (exit.cost.clone(), exit.to.clone(), exit.name.clone()));
    if let Some((cost, _, _)) = &travel {
        cost::pay(draft, &intent.actor_id, cost)?;
    }
    let until = world_time_before.saturating_add(minutes);
    let pending = Pending {
        rule: rule.id.clone(),
        verb: intent.move_name.clone(),
        target: match &intent.target {
            Target::Place(id) | Target::Actor(id) | Target::Clock(id) => id.clone(),
            Target::Empty => String::new(),
        },
        arrive: travel.as_ref().map(|(_, to, _)| to.clone()),
        via: travel.as_ref().map(|(_, _, name)| name.clone()),
        source: match intent.source {
            Source::Kernel => "kernel".to_string(),
            Source::Player => "player".to_string(),
            Source::System => "system".to_string(),
        },
    };
    draft.track(StateKey::ActorBusy(intent.actor_id.clone()))?;
    let act = draft
        .now_mut()
        .actors
        .get_mut(&intent.actor_id)
        .ok_or(Reason::UnknownKey)?
        .act_mut();
    act.busy_until = Some(until);
    act.busy_for = Some(pending);
    Ok(())
}

/// Places no exit path reaches from the start, including hidden and locked exits.
pub fn unreachable(canon: &Canon) -> Vec<Diagnostic> {
    let mut seen = std::collections::BTreeSet::new();
    let mut queue = vec![canon.start_place.clone()];
    while let Some(place) = queue.pop() {
        if !seen.insert(place.clone()) {
            continue;
        }
        if let Some(node) = canon.places.get(&place) {
            for exit in &node.exits {
                queue.push(exit.to.clone());
            }
        }
    }
    let mut diags = Vec::new();
    for id in canon.places.keys() {
        if seen.contains(id) {
            continue;
        }
        diags.push(
            Diagnostic::new(
                "world/places",
                "WARNING",
                &format!("place '{id}' has no path from {}", canon.start_place),
                "add an exit that reaches it",
            )
            .with_path(id.clone()),
        );
    }
    diags
}

/// `teleport: true` in the shared verbs file is a warning. Author rules may teleport.
pub(crate) fn teleport_in_verbs(canon: &Canon) -> Vec<Diagnostic> {
    canon
        .rules
        .iter()
        .filter(|rule| {
            rule.file.replace('\\', "/").ends_with("rules/verbs.yaml")
                && crate::canon::schema::uses_teleport(rule)
        })
        .map(|rule| {
            Diagnostic::new(
                &rule.file,
                "WARNING",
                &format!("verb '{}' teleports", rule.local_id),
                "teleport belongs on an author rule, not in verbs.yaml",
            )
            .with_path(format!("rules[{}]", rule.index))
        })
        .collect()
}

pub fn player_lines(canon: &Canon, now: &Now, actor_id: &str) -> String {
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
            if !exit::revealed(canon, now, actor_id, exit) {
                continue;
            }
            lines.push(format!("  {}", exit_line(canon, now, actor_id, exit)));
        }
    }
    lines.join("\n")
}

/// Verbs whose `if` holds for this actor, then the visible exits, then a busy line.
pub fn help_lines(canon: &Canon, now: &Now, actor_id: &str) -> String {
    let mut lines = open_verbs(canon, now, actor_id);
    lines.extend(exit_phrases(canon, now, actor_id));
    if let Some(status) = busy_line(now, actor_id) {
        lines.push(status);
    }
    lines.join("\n")
}

/// The place name, the prose this actor may read, and the visible exits.
pub fn look_lines(canon: &Canon, now: &Now, actor_id: &str) -> String {
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
    let exits = exit_phrases(canon, now, actor_id).join("\n");
    let head = head.join("\n");
    if head.is_empty() {
        exits
    } else {
        format!("{head}\n\n{exits}")
    }
}

/// `on the stairs until 21:40` while a journey is still running.
pub fn busy_line(now: &Now, actor_id: &str) -> Option<String> {
    let actor = now.actors.get(actor_id)?;
    if !actor.is_busy(now.world_time) {
        return None;
    }
    let until = actor.act().busy_until?;
    let via = actor
        .act()
        .busy_for
        .as_ref()
        .and_then(|pending| pending.via.as_deref())
        .filter(|name| !name.is_empty())?;
    Some(format!("on the {via} until {}", clock_face(until)))
}

/// Place prose with hidden fact text this actor cannot read removed.
pub fn place_prose(canon: &Canon, now: &Now, actor_id: &str) -> String {
    let Some(actor) = now.actors.get(actor_id) else {
        return String::new();
    };
    let Some(place) = canon.places.get(actor.place()) else {
        return String::new();
    };
    let kind = if actor.is_pc() {
        ActorKind::Player
    } else {
        ActorKind::Npc
    };
    let Ok(viewer) = ViewerId::from_kind(kind, actor_id) else {
        return String::new();
    };
    let mut body = place.body.clone();
    for (id, fact) in &canon.facts {
        if fact.text.is_empty() || allows(canon, now, &viewer, id) {
            continue;
        }
        if body.contains(fact.text.as_str()) {
            body = body.replace(fact.text.as_str(), "");
        }
    }
    body.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn cast_name(canon: &Canon, id: &str) -> String {
    canon
        .cast
        .get(id)
        .map(|cast| cast.display_name.clone())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| id.to_string())
}

pub fn exit_phrase(name: &str, minutes: u64, locked: bool) -> String {
    if locked {
        format!("{name} (locked) — {minutes} min")
    } else {
        format!("{name} — {minutes} min")
    }
}

/// Minutes from midnight as `21:40`.
pub fn clock_face(minutes: u64) -> String {
    let hour = (minutes / 60) % 24;
    let minute = minutes % 60;
    format!("{hour:02}:{minute:02}")
}

fn exit_phrases(canon: &Canon, now: &Now, actor_id: &str) -> Vec<String> {
    let views = visible(canon, now, actor_id);
    if views.is_empty() {
        return vec!["Exits:".to_string(), "none".to_string()];
    }
    let mut lines = vec!["Exits:".to_string()];
    for view in views {
        lines.push(exit_phrase(&view.name, view.minutes, view.locked));
    }
    lines
}

fn open_verbs(canon: &Canon, now: &Now, actor_id: &str) -> Vec<String> {
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
        if verb_open(canon, now, actor_id, verb, *target) {
            names.push(verb.clone());
        }
    }
    names
}

fn verb_open(canon: &Canon, now: &Now, actor_id: &str, verb: &str, target: TargetKind) -> bool {
    let Some(actor) = now.actors.get(actor_id) else {
        return false;
    };
    let place = actor.place().to_string();
    let wielded = actor.wielded().map(str::to_string);
    let kind = if actor.is_pc() {
        ActorKind::Player
    } else {
        ActorKind::Npc
    };
    for hint in verb_hints(canon, now, actor_id, target) {
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
        let intent = Intent {
            id: String::new(),
            tick_index: now.tick_index,
            actor_id: actor_id.to_string(),
            actor_kind: kind,
            source: Source::Player,
            move_name: verb.to_string(),
            target: chosen.store(&token),
            args: IntentArgs::default(),
            rationale_code: String::new(),
        };
        if cond::holds(canon, now, &Ctx { intent: &intent }, &rule.if_).is_ok() {
            return true;
        }
    }
    false
}

fn verb_hints(canon: &Canon, now: &Now, actor_id: &str, target: TargetKind) -> Vec<Option<String>> {
    let hints = match target {
        TargetKind::None => Vec::new(),
        TargetKind::Exit => visible(canon, now, actor_id)
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

pub fn graph(canon: &Canon) -> String {
    let mut text = String::from("exits\n");
    let mut mermaid = String::from("flowchart LR\n");
    for (id, place) in &canon.places {
        text.push_str(&place_name(canon, id));
        text.push('\n');
        for exit in &place.exits {
            text.push_str(&format!(
                "  {} -> {} ({})\n",
                exit.name,
                exit.to,
                minutes_label(exit.minutes)
            ));
            mermaid.push_str(&format!(
                "  {id} -->|{}| {}\n",
                exit.name.replace('|', "/"),
                exit.to
            ));
        }
    }
    format!("{text}\n```mermaid\n{mermaid}```\n")
}

fn exit_line(canon: &Canon, now: &Now, actor_id: &str, exit: &Exit) -> String {
    exit_phrase(
        &exit.name,
        exit.minutes,
        exit::locked(canon, now, actor_id, exit),
    )
}

fn minutes_label(minutes: u64) -> String {
    if minutes == 0 {
        "0 min".to_string()
    } else {
        format!("{minutes} min")
    }
}

pub fn place_name(canon: &Canon, id: &str) -> String {
    canon
        .places
        .get(id)
        .map(|place| {
            if place.name.is_empty() {
                id.to_string()
            } else {
                place.name.clone()
            }
        })
        .unwrap_or_else(|| id.to_string())
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
        let now = crate::store::codec::load_save(
            &fs::read_to_string(root.join("state/save.yaml")).unwrap(),
        )
        .unwrap();
        let help = help_lines(&canon, &now, "pc_rowan");
        assert!(help.lines().any(|line| line == "wait"), "{help}");
        assert!(help.lines().any(|line| line == "travel"), "{help}");
        assert!(!help.lines().any(|line| line == "use"), "{help}");
        assert!(help.contains("stairs (locked) — 2 min"), "{help}");
        assert!(help.contains("docks_street — 0 min"), "{help}");
        assert!(!help.contains("→"), "{help}");
        let look = look_lines(&canon, &now, "pc_rowan");
        assert!(look.contains("Docks Shop"), "{look}");
        assert!(
            look.contains("The shop faces the water. Tins sit in a row."),
            "{look}"
        );
        assert!(look.contains("stairs (locked) — 2 min"), "{look}");
    }
}
