use spookery_kernel::limits::Budget;
use spookery_kernel::Reason;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Catchup {
    pub downtime_hours: Option<u64>,
    pub skipped: u64,
    pub last: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CatchupPlan {
    pub ticks_to_run: u64,
    pub skipped: u64,
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

pub fn plan_catchup(
    humans: u32,
    budget: &Budget,
    downtime_hours: u64,
) -> Result<CatchupPlan, Reason> {
    if humans > 0 {
        return Ok(CatchupPlan {
            ticks_to_run: 1,
            skipped: 0,
        });
    }
    let every = budget.tick_every_real_seconds;
    let idle_minutes = budget.idle_minutes_per_tick;
    if every == 0 || idle_minutes == 0 {
        return Err(Reason::Schema);
    }
    let per_hour = downtime_hours.checked_mul(60).ok_or(Reason::OutOfRange)?;
    let printed = ceil_div(per_hour, every);
    let minute_cap = budget
        .idle_max_world_hours_per_real_hour
        .checked_mul(per_hour)
        .ok_or(Reason::OutOfRange)?;
    let by_minutes = minute_cap / idle_minutes;
    let ticks_to_run = printed.min(budget.idle_catchup_max_ticks).min(by_minutes);
    Ok(CatchupPlan {
        ticks_to_run,
        skipped: printed.saturating_sub(ticks_to_run),
    })
}

fn ceil_div(numerator: u64, denominator: u64) -> u64 {
    if numerator == 0 {
        return 0;
    }
    let quotient = numerator / denominator;
    if numerator % denominator == 0 {
        quotient
    } else {
        quotient.saturating_add(1)
    }
}
