use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use spookery_kernel::shard::{open_shard, Shard};
use spookery_kernel::Reason;
use spookery_tick::advance::plan_catchup;
use spookery_tick::{run_catchup, run_tick, Catchup, Reason as TickReason};

pub async fn run_wake(pack: &Path) -> Result<(), Reason> {
    let mut shard = open_shard(pack)?;
    let seconds = shard.budget().tick_every_real_seconds;
    tokio::time::sleep(Duration::from_secs(seconds)).await;
    wake_shard(&mut shard)
}

pub async fn run_scheduler(pack: &Path) -> Result<(), Reason> {
    loop {
        run_wake(pack).await?;
    }
}

pub fn seconds_since(last_tick_at: &str) -> Result<u64, Reason> {
    elapsed_secs(last_tick_at, SystemTime::now())
}

pub fn wall_now() -> Result<String, Reason> {
    format_utc(SystemTime::now())
}

pub fn wake_shard(shard: &mut Shard) -> Result<(), Reason> {
    let now = SystemTime::now();
    let elapsed = elapsed_secs(&shard.now().last_tick_at, now)?;
    let wall = format_utc(now)?;
    let hours = elapsed / 3600;
    if hours == 0 {
        run_tick(shard, TickReason::Scheduler, &wall, Catchup::none())
    } else {
        let humans = shard.now().presence.humans;
        let plan = plan_catchup(humans, shard.budget(), hours)?;
        if plan.ticks_to_run == 0 {
            run_tick(shard, TickReason::Scheduler, &wall, Catchup::none())
        } else {
            run_catchup(shard, &wall, hours)
        }
    }
}

fn elapsed_secs(last_tick_at: &str, now: SystemTime) -> Result<u64, Reason> {
    let last = parse_utc(last_tick_at)?;
    let now = now
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Reason::Schema)?
        .as_secs();
    Ok(now.saturating_sub(last))
}

fn parse_utc(text: &str) -> Result<u64, Reason> {
    let bytes = text.as_bytes();
    if bytes.len() != 20 || bytes[10] != b'T' || bytes[19] != b'Z' {
        return Err(Reason::Schema);
    }
    let year = number(text, 0, 4)?;
    let month = number(text, 5, 7)?;
    let day = number(text, 8, 10)?;
    let hour = number(text, 11, 13)?;
    let minute = number(text, 14, 16)?;
    let second = number(text, 17, 19)?;
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return Err(Reason::Schema);
    }
    let days = days_from_civil(year as i64, month as i64, day as i64);
    let secs = days
        .checked_mul(86400)
        .and_then(|days| days.checked_add(hour as i64 * 3600))
        .and_then(|days| days.checked_add(minute as i64 * 60))
        .and_then(|days| days.checked_add(second as i64))
        .ok_or(Reason::Schema)?;
    u64::try_from(secs).map_err(|_| Reason::Schema)
}

fn format_utc(now: SystemTime) -> Result<String, Reason> {
    let secs = now
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Reason::Schema)?
        .as_secs();
    let secs = i64::try_from(secs).map_err(|_| Reason::Schema)?;
    let days = div_floor(secs, 86400);
    let tod = secs - days * 86400;
    let (year, month, day) = civil_from_days(days);
    let hour = tod / 3600;
    let minute = (tod % 3600) / 60;
    let second = tod % 60;
    Ok(format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z"
    ))
}

fn days_from_civil(mut year: i64, month: i64, day: i64) -> i64 {
    year -= i64::from(month <= 2);
    let era = div_floor(year, 400);
    let yoe = year - era * 400;
    let mp = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719468;
    let era = div_floor(z, 146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = y + i64::from(month <= 2);
    (year, month, day)
}

fn div_floor(a: i64, b: i64) -> i64 {
    let d = a / b;
    let r = a % b;
    if r != 0 && (r < 0) != (b < 0) {
        d - 1
    } else {
        d
    }
}

fn number(text: &str, start: usize, end: usize) -> Result<u32, Reason> {
    text.get(start..end)
        .and_then(|slice| slice.parse().ok())
        .ok_or(Reason::Schema)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_stamps_round_trip() {
        assert_eq!(parse_utc("1970-01-01T00:00:00Z").unwrap(), 0);
        assert_eq!(parse_utc("2026-01-01T00:00:00Z").unwrap(), 1_767_225_600);
        let docks = UNIX_EPOCH + Duration::from_secs(1_767_225_600);
        assert_eq!(format_utc(docks).unwrap(), "2026-01-01T00:00:00Z");
    }
}
