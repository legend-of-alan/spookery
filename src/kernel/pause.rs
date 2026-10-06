use crate::error::Reason;
use crate::key::StateKey;
use crate::limits::PausePolicy;
use crate::model::RunState;
use crate::shard::Txn;

pub fn apply_pause_policy(txn: &mut Txn) -> Result<(), Reason> {
    let humans = txn.now().presence.humans;
    let policies = txn.budget().pause;
    let clock_ids: Vec<String> = txn.now().clocks.keys().cloned().collect();
    for clock_id in clock_ids {
        let Some(clock) = txn.now().clocks.get(&clock_id) else {
            continue;
        };
        if !clock.requires_presence {
            continue;
        }
        let paused = humans == 0 && policies.named_plot_clocks == PausePolicy::PauseWhenEmpty;
        if humans > 0 {
            set_paused(txn, &clock_id, false)?;
        } else if paused {
            set_paused(txn, &clock_id, true)?;
        }
    }
    let mut view = txn.now().pause.clone();
    view.named_plot_clocks = view_state(policies.named_plot_clocks, humans);
    txn.set_pause_view(view);
    Ok(())
}

fn set_paused(txn: &mut Txn, clock_id: &str, paused: bool) -> Result<(), Reason> {
    let current = txn.now().clocks.get(clock_id).map(|clock| clock.paused);
    if current != Some(paused) {
        txn.track(StateKey::ClockPaused(clock_id.to_string()))?;
    }
    if let Some(clock) = txn.now_mut().clocks.get_mut(clock_id) {
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
