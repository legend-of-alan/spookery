//! Decide when an idle table plays a round, and stamp the wall time.

use chrono::{DateTime, NaiveDateTime, TimeDelta, Utc};

use spookery_engine::{Reason, World};

const STAMP: &str = "%Y-%m-%dT%H:%M:%SZ";

/// Test support: seconds since a UTC stamp. Not part of the API.
#[doc(hidden)]
pub fn seconds_since(last_round_at: &str) -> Result<u64, Reason> {
    elapsed_secs(last_round_at, Utc::now())
}

pub fn wall_now() -> String {
    format_utc(Utc::now())
}

/// Test support: a UTC stamp `seconds` in the past. Not part of the API.
#[doc(hidden)]
pub fn wall_ago(seconds: u64) -> Result<String, Reason> {
    let ago = i64::try_from(seconds)
        .ok()
        .and_then(TimeDelta::try_seconds)
        .and_then(|ago| Utc::now().checked_sub_signed(ago))
        .ok_or(Reason::Schema)?;
    Ok(format_utc(ago))
}

fn elapsed_secs(last_round_at: &str, now: DateTime<Utc>) -> Result<u64, Reason> {
    let last = parse_utc(last_round_at)?;
    Ok(u64::try_from((now - last).num_seconds()).unwrap_or(0))
}

fn parse_utc(text: &str) -> Result<DateTime<Utc>, Reason> {
    if text.len() != 20 {
        return Err(Reason::Schema);
    }
    NaiveDateTime::parse_from_str(text, STAMP)
        .map(|stamp| stamp.and_utc())
        .map_err(|_| Reason::Schema)
}

fn format_utc(now: DateTime<Utc>) -> String {
    now.format(STAMP).to_string()
}

/// Seat check and interval check. A round that runs borrows this `World`.
/// Returns whether `world_time` advanced.
pub fn on_scheduled_wake(
    world: &mut World,
    player_held: bool,
    gm_held: bool,
) -> Result<bool, Reason> {
    if player_held || gm_held {
        return Ok(false);
    }
    if !world.budget().idle_rounds {
        return Ok(false);
    }
    let now = Utc::now();
    let elapsed = elapsed_secs(&world.state().last_round_at, now)?;
    if elapsed < world.budget().round_every_real_seconds {
        return Ok(false);
    }
    let before = world.state().world_time;
    world.catch_up(&format_utc(now), elapsed / 3600)?;
    Ok(world.state().world_time != before)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_stamps_round_trip() {
        assert_eq!(parse_utc("1970-01-01T00:00:00Z").unwrap().timestamp(), 0);
        let docks = parse_utc("2026-01-01T00:00:00Z").unwrap();
        assert_eq!(docks.timestamp(), 1_767_225_600);
        assert_eq!(format_utc(docks), "2026-01-01T00:00:00Z");
    }
}
