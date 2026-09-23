use spookery_kernel::limits::Budget;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Catchup {
    pub downtime_hours: Option<u64>,
    pub skipped: u64,
    pub last: bool,
}

impl Catchup {
    pub fn none() -> Self {
        Self {
            downtime_hours: None,
            skipped: 0,
            last: false,
        }
    }
}

pub fn pick_advance(humans: u32, _catchup: &Catchup, budget: &Budget) -> u64 {
    if humans > 0 {
        budget.live_minutes_per_tick
    } else {
        budget.idle_minutes_per_tick
    }
}
