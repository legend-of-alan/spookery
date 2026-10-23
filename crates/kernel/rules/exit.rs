//! Exit admission. Rules ask this while gating; the map still draws the journey.

use crate::canon::{Canon, Exit};
use crate::error::Reason;
use crate::firewall::allows;
use crate::intent::{Intent, IntentArgs, Source, Target};
use crate::model::Now;
use crate::rule::{EffectTemplate, LocationTo, Rule};
use crate::rules::cond;
use crate::rules::cost;
use crate::rules::expr::Ctx;
use crate::rules::gate::{self, GateRefusal};

pub(crate) fn named<'a>(canon: &'a Canon, from: &str, name: &str) -> Option<&'a Exit> {
    canon
        .places
        .get(from)
        .and_then(|place| place.exits.iter().find(|exit| exit.name == name))
}

/// The exit a `location via` effect names, if this rule travels.
pub(crate) fn token(rule: &Rule, intent: &Intent) -> Option<String> {
    for effect in rule.top_effects() {
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
pub(crate) fn admit(canon: &Canon, now: &Now, intent: &Intent) -> Result<(), GateRefusal> {
    let Ok(rule) = gate::resolve(canon, now, intent) else {
        return Ok(());
    };
    let Some(token) = token(rule, intent) else {
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
        return Err(GateRefusal::Text {
            code: Reason::NoExit,
            text,
        });
    }
    if let Err(shortfall) = cost::can_pay(canon, now, intent, &exit.cost) {
        return Err(GateRefusal::CantAfford(shortfall.missing));
    }
    Ok(())
}

pub(crate) fn revealed(canon: &Canon, now: &Now, actor_id: &str, exit: &Exit) -> bool {
    if !exit.hidden {
        return true;
    }
    let Some(fact) = &exit.revealed_by else {
        return false;
    };
    let Some(viewer) = now.viewer_of(actor_id) else {
        return false;
    };
    allows(canon, now, &viewer, fact)
}

pub(crate) fn locked(canon: &Canon, now: &Now, actor_id: &str, exit: &Exit) -> bool {
    if exit.if_.is_empty() {
        return false;
    }
    let Some(actor) = now.actors.get(actor_id) else {
        return true;
    };
    let intent = Intent {
        id: String::new(),
        tick_index: now.tick_index,
        actor_id: actor_id.to_string(),
        actor_kind: actor.actor_kind(),
        source: Source::Player,
        move_name: "travel".to_string(),
        target: Target::Place(exit.name.clone()),
        args: IntentArgs::default(),
        rationale_code: String::new(),
    };
    cond::holds(canon, now, &Ctx { intent: &intent }, &exit.if_).is_err()
}
