//! Exits and the journeys that follow them.

use crate::canon::Canon;
use crate::diagnostic::{schema, Diagnostic};
use crate::error::Reason;
use crate::firewall::allows;
use crate::intent::{ActorKind, Intent, IntentArgs, Source, Target, ViewerId};
use crate::key::StateKey;
use crate::model::{Now, Pending};
use crate::rules::cond;
use crate::rules::cost;
use crate::rules::expr::Ctx;
use crate::rules::gate::Miss;
use crate::rules::rule::{Cost, EffectTemplate, LocationTo, Rule};
use crate::world::Draft;

/// One way out of a place. `minutes` is the world time the walk takes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exit {
    pub name: String,
    pub to: String,
    pub minutes: u64,
    pub cost: Cost,
    pub if_: Vec<crate::rules::rule::Cond>,
    pub locked_text: String,
    pub hidden: bool,
    pub revealed_by: Option<String>,
    pub file: String,
    pub index: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExitView {
    pub name: String,
    pub to: String,
    pub place_name: String,
    pub minutes: u64,
    pub locked: bool,
}

pub fn named<'a>(canon: &'a Canon, from: &str, name: &str) -> Option<&'a Exit> {
    canon
        .places
        .get(from)
        .and_then(|place| place.exits.iter().find(|exit| exit.name == name))
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
        .filter(|exit| revealed(canon, now, actor_id, exit))
        .map(|exit| ExitView {
            name: exit.name.clone(),
            to: exit.to.clone(),
            place_name: place_name(canon, &exit.to),
            minutes: exit.minutes,
            locked: locked(canon, now, actor_id, exit),
        })
        .collect()
}

/// The exit a `location via` effect names, if this rule travels.
pub fn exit_token(rule: &Rule, intent: &Intent) -> Option<String> {
    for effect in rule
        .do_
        .iter()
        .chain(rule.success.iter())
        .chain(rule.partial.iter())
        .chain(rule.failure.iter())
    {
        if let EffectTemplate::Location {
            to: LocationTo::Via(via),
            teleport: false,
        } = effect
        {
            if via == "target" {
                if let Target::Place(name) = &intent.target {
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
pub fn admit(canon: &Canon, now: &Now, intent: &Intent) -> Result<(), Miss> {
    let Ok(rule) = crate::rules::gate::resolve(canon, now, intent) else {
        return Ok(());
    };
    let Some(token) = exit_token(rule, intent) else {
        return Ok(());
    };
    let from = now
        .actors
        .get(&intent.actor_id)
        .map(|actor| actor.place())
        .unwrap_or("");
    let Some(exit) = named(canon, from, &token) else {
        return Err(Reason::NoExit.into());
    };
    if !revealed(canon, now, &intent.actor_id, exit) {
        return Err(Reason::NoExit.into());
    }
    if locked(canon, now, &intent.actor_id, exit) {
        let text = if exit.locked_text.is_empty() {
            "The way is locked.".to_string()
        } else {
            exit.locked_text.clone()
        };
        return Err(Miss::Text {
            code: Reason::NoExit,
            text,
        });
    }
    if let Err(gap) = cost::can_pay(canon, now, &intent.actor_id, &exit.cost) {
        return Err(Miss::CantAfford(gap.what));
    }
    Ok(())
}

/// World minutes until the rule and its exit finish. Zero means it finishes now.
pub fn span(canon: &Canon, now: &Now, intent: &Intent, rule: &Rule) -> u64 {
    let mut minutes = rule.cost.as_ref().map(|cost| cost.minutes).unwrap_or(0);
    if let Some(exit) = used_exit(canon, now, intent, rule) {
        minutes = minutes.saturating_add(exit.minutes);
    }
    minutes
}

pub fn used_exit<'a>(
    canon: &'a Canon,
    now: &Now,
    intent: &Intent,
    rule: &Rule,
) -> Option<&'a Exit> {
    let token = exit_token(rule, intent)?;
    let from = now.actors.get(&intent.actor_id)?.place();
    named(canon, from, &token)
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
            Source::Player | Source::System => "player".to_string(),
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

/// Exits whose `to` is not a place, and facts or items an exit names but the pack does not.
pub fn broken(canon: &Canon) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    for place in canon.places.values() {
        for exit in &place.exits {
            if !canon.places.contains_key(&exit.to) {
                diags.push(
                    schema(
                        &exit.file,
                        &format!(
                            "exit '{}' goes to '{}', which is not a place",
                            exit.name, exit.to
                        ),
                        "set to: to a place id",
                    )
                    .with_path(format!("exits[{}].to", exit.index)),
                );
            }
            if exit.hidden && exit.revealed_by.is_none() {
                diags.push(
                    schema(
                        &exit.file,
                        &format!("hidden exit '{}' has no revealed_by fact", exit.name),
                        "set revealed_by to a fact id",
                    )
                    .with_path(format!("exits[{}].revealed_by", exit.index)),
                );
            }
            if let Some(fact) = &exit.revealed_by {
                if !canon.facts.contains_key(fact) {
                    diags.push(
                        schema(
                            &exit.file,
                            &format!("exit '{}' reveals '{fact}', which is not a fact", exit.name),
                            "use a fact id from this pack",
                        )
                        .with_path(format!("exits[{}].revealed_by", exit.index)),
                    );
                }
            }
            let mut items = Vec::new();
            for (id, _) in &exit.cost.items {
                if id != "target" {
                    items.push(id.clone());
                }
            }
            for cond in &exit.if_ {
                cond_items(cond, &mut items);
            }
            items.sort();
            items.dedup();
            for id in items {
                if !canon.items.contains_key(&id) {
                    diags.push(
                        schema(
                            &exit.file,
                            &format!("exit '{}' names unknown item '{id}'", exit.name),
                            &format!("add world/items/{id}.md"),
                        )
                        .with_path(format!("exits[{}]", exit.index)),
                    );
                }
            }
        }
    }
    diags
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
pub fn teleport_in_verbs(canon: &Canon) -> Vec<Diagnostic> {
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
            if !revealed(canon, now, actor_id, exit) {
                continue;
            }
            lines.push(format!("  {}", exit_line(canon, now, actor_id, exit)));
        }
    }
    lines.join("\n")
}

pub fn help_lines(canon: &Canon, now: &Now, actor_id: &str) -> String {
    let views = visible(canon, now, actor_id);
    if views.is_empty() {
        return "Exits:\nnone".to_string();
    }
    let mut lines = vec!["Exits:".to_string()];
    for view in views {
        lines.push(format_view(&view));
    }
    lines.join("\n")
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

fn format_view(view: &ExitView) -> String {
    let mut line = format!("{} → {}", view.name, view.place_name);
    let mut notes = Vec::new();
    if view.minutes > 0 {
        notes.push(format!("{} min", view.minutes));
    }
    if view.locked {
        notes.push("locked".to_string());
    }
    if !notes.is_empty() {
        line.push_str(&format!(" ({})", notes.join(", ")));
    }
    line
}

fn exit_line(canon: &Canon, now: &Now, actor_id: &str, exit: &Exit) -> String {
    format_view(&ExitView {
        name: exit.name.clone(),
        to: exit.to.clone(),
        place_name: place_name(canon, &exit.to),
        minutes: exit.minutes,
        locked: locked(canon, now, actor_id, exit),
    })
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

fn revealed(canon: &Canon, now: &Now, actor_id: &str, exit: &Exit) -> bool {
    if !exit.hidden {
        return true;
    }
    let Some(fact) = &exit.revealed_by else {
        return false;
    };
    let Some(actor) = now.actors.get(actor_id) else {
        return false;
    };
    let kind = if actor.is_pc() {
        ActorKind::Player
    } else {
        ActorKind::Npc
    };
    let Ok(viewer) = ViewerId::from_kind(kind, actor_id) else {
        return false;
    };
    allows(canon, now, &viewer, fact)
}

fn locked(canon: &Canon, now: &Now, actor_id: &str, exit: &Exit) -> bool {
    if exit.if_.is_empty() {
        return false;
    }
    let Some(actor) = now.actors.get(actor_id) else {
        return true;
    };
    let kind = if actor.is_pc() {
        ActorKind::Player
    } else {
        ActorKind::Npc
    };
    let intent = Intent {
        id: String::new(),
        tick_index: now.tick_index,
        actor_id: actor_id.to_string(),
        actor_kind: kind,
        source: Source::Player,
        move_name: "travel".to_string(),
        target: Target::Place(exit.name.clone()),
        args: IntentArgs::default(),
        rationale_code: String::new(),
    };
    cond::holds(canon, now, &Ctx { intent: &intent }, &exit.if_).is_err()
}

fn cond_items(cond: &crate::rules::rule::Cond, ids: &mut Vec<String>) {
    use crate::rules::rule::Cond;
    match cond {
        Cond::HasItem { id, .. } if id != "target" => ids.push(id.clone()),
        Cond::All(inner) | Cond::Any(inner) => {
            for cond in inner {
                cond_items(cond, ids);
            }
        }
        Cond::Not(inner) => cond_items(inner, ids),
        _ => {}
    }
}
