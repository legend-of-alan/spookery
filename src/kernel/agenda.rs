use crate::canon::{AgendaEntry, Canon, PauseLane, ScheduleGate};
use crate::effect::{Effect, EffectClass};
use crate::eligibility::is_eligible;
use crate::error::Reason;
use crate::expand::expand_outcome;
use crate::ids::kernel_intent_id;
use crate::intent::{ActorKind, Intent, IntentArgs, Source, Target};
use crate::limits::world_hour;
use crate::model::{Now, RunState};
use crate::moves::Audience;
use crate::select::{first_drift, first_due_note, first_owned_clock, rumor_class, stock_walk};

/// First agenda entry whose gates pass and whose row expands is the NPC's intent.
pub fn kernel_intents_for_eligible_npcs(canon: &Canon, now: &Now) -> Vec<Intent> {
    let mut intents = Vec::new();
    for actor_id in now.actors.keys() {
        if !is_eligible(canon, now, actor_id) {
            continue;
        }
        if let Some(intent) = select_one(canon, now, actor_id) {
            intents.push(intent);
        }
    }
    intents
}

pub fn allowed(canon: &Canon, now: &Now, intent: &Intent) -> Result<(), Reason> {
    if !canon.moves.contains_key(&intent.move_name) {
        return Err(Reason::UnknownMove);
    }
    match &intent.target {
        Target::Empty => {}
        Target::Place(id) => {
            if !canon.places.contains_key(id) && !now.places.contains_key(id) {
                return Err(Reason::UnknownKey);
            }
        }
        Target::Actor(id) => {
            if !now.actors.contains_key(id) {
                return Err(Reason::UnknownKey);
            }
        }
        Target::Clock(id) => {
            if !now.clocks.contains_key(id) {
                return Err(Reason::UnknownKey);
            }
        }
        Target::Rumor(id) => {
            if !now.rumors.iter().any(|rumor| rumor.id == *id) {
                return Err(Reason::UnknownKey);
            }
        }
    }
    if intent.source == Source::Kernel && intent.actor_kind == ActorKind::Player {
        return Err(Reason::KernelOnPlayer);
    }
    if intent.source == Source::Kernel
        && intent.actor_kind == ActorKind::Npc
        && !is_eligible(canon, now, &intent.actor_id)
    {
        return Err(Reason::Ineligible);
    }
    Ok(())
}

fn select_one(canon: &Canon, now: &Now, actor_id: &str) -> Option<Intent> {
    let profile = canon.profile(actor_id)?;
    for entry in &profile.agenda {
        if !gates(now, profile, entry, actor_id)
            || !when_ready(canon, now, profile, entry, actor_id)
        {
            continue;
        }
        let row = canon.moves.get(&entry.row)?.row.as_ref()?;
        if row.audience != Some(Audience::Agenda) {
            continue;
        }
        let mut intent = mint(now, actor_id, &entry.row);
        match expand_outcome(canon, now, &intent, None) {
            Ok(effects) => {
                stamp(&mut intent, &effects);
                return Some(intent);
            }
            Err(_) => continue,
        }
    }
    None
}

fn gates(
    now: &Now,
    profile: &crate::canon::TickProfile,
    entry: &AgendaEntry,
    actor_id: &str,
) -> bool {
    if let Some(lane) = entry.pause {
        let state = match lane {
            PauseLane::NamedPlotClocks => now.pause.named_plot_clocks,
            PauseLane::SubsistenceClocks => now.pause.subsistence_clocks,
            PauseLane::RelationshipDrift => now.pause.relationship_drift,
            PauseLane::Travel => now.pause.travel,
        };
        if state != RunState::Running {
            return false;
        }
    }
    if entry.schedule.is_none() && entry.activity.is_none() {
        return true;
    }
    let Some(actor) = now.actors.get(actor_id) else {
        return false;
    };
    let Some(row) = profile.winning_row(world_hour(now.world_time)) else {
        return false;
    };
    match entry.schedule {
        Some(ScheduleGate::Away) => {
            row.place != actor.place() && profile.range.iter().any(|place| place == &row.place)
        }
        Some(ScheduleGate::Here) => {
            row.place == actor.place() && entry.activity.as_deref() == row.activity.as_deref()
        }
        None => false,
    }
}

fn when_ready(
    canon: &Canon,
    now: &Now,
    profile: &crate::canon::TickProfile,
    entry: &AgendaEntry,
    actor_id: &str,
) -> bool {
    if entry.when_stock && stock_walk(canon, now, actor_id).is_none() {
        return false;
    }
    if entry.when_drift && first_drift(now, actor_id).is_none() {
        return false;
    }
    if entry.when_rumor && rumor_class(canon, now, actor_id).is_none() {
        return false;
    }
    if entry.when_note && first_due_note(now, profile).is_none() {
        return false;
    }
    if entry.when_clock && first_owned_clock(now, actor_id).is_none() {
        return false;
    }
    true
}

fn mint(now: &Now, actor_id: &str, row: &str) -> Intent {
    Intent {
        id: kernel_intent_id(&now.shard_id, now.tick_index, actor_id, row),
        tick_index: now.tick_index,
        actor_id: actor_id.to_string(),
        actor_kind: ActorKind::Npc,
        source: Source::Kernel,
        move_name: row.to_string(),
        target: Target::Empty,
        args: IntentArgs::default(),
        rationale_code: String::new(),
    }
}

fn stamp(intent: &mut Intent, effects: &[Effect]) {
    for effect in effects {
        match &effect.class {
            EffectClass::Location { to, .. } => {
                intent.target = Target::Place(to.clone());
            }
            EffectClass::Relationship { b, .. } => {
                intent.target = Target::Actor(b.clone());
            }
            EffectClass::Rumor { rumor_id, .. } => {
                intent.target = Target::Rumor(rumor_id.clone());
            }
            EffectClass::Clock { clock_id, .. } => {
                intent.target = Target::Clock(clock_id.clone());
            }
            EffectClass::Message { to, .. } => {
                intent.args.message_to = Some(to.clone());
            }
            _ => {}
        }
    }
}
