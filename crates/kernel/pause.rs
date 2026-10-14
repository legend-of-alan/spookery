use crate::canon::world_settings::PausePolicy;
use crate::error::Reason;
use crate::key::StateKey;
use crate::model::RunState;
use crate::world::Draft;

pub fn apply_pause_policy(draft: &mut Draft) -> Result<(), Reason> {
    let humans = draft.now().presence.humans;
    let policies = draft.budget().pause;
    let clock_ids: Vec<String> = draft.now().clocks.keys().cloned().collect();
    for clock_id in clock_ids {
        let Some(clock) = draft.now().clocks.get(&clock_id) else {
            continue;
        };
        if !clock.requires_presence {
            continue;
        }
        let paused = humans == 0 && policies.named_plot_clocks == PausePolicy::PauseWhenEmpty;
        if humans > 0 {
            set_paused(draft, &clock_id, false)?;
        } else if paused {
            set_paused(draft, &clock_id, true)?;
        }
    }
    let mut view = draft.now().pause.clone();
    view.named_plot_clocks = view_state(policies.named_plot_clocks, humans);
    draft.set_pause_view(view);
    Ok(())
}

fn set_paused(draft: &mut Draft, clock_id: &str, paused: bool) -> Result<(), Reason> {
    let current = draft.now().clocks.get(clock_id).map(|clock| clock.paused);
    if current != Some(paused) {
        draft.track(StateKey::ClockPaused(clock_id.to_string()))?;
    }
    if let Some(clock) = draft.now_mut().clocks.get_mut(clock_id) {
        clock.paused = paused;
    }
    Ok(())
}

fn view_state(policy: PausePolicy, humans: u32) -> RunState {
    match policy {
        PausePolicy::PauseWhenEmpty if humans == 0 => RunState::Paused,
        PausePolicy::PauseWhenEmpty | PausePolicy::AlwaysRun => RunState::Running,
    }
}
