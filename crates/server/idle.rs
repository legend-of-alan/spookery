//! The idle timer, when an idle table plays a round, and the wall time stamp.

use std::sync::mpsc::Sender;
use std::time::Duration;

use chrono::{DateTime, NaiveDateTime, Utc};
use tokio::sync::watch;

use spookery_engine::{Reason, World};

use super::table::Request;

const STAMP_FORMAT: &str = "%Y-%m-%dT%H:%M:%SZ";

/// Wakes the table every `keepalive_secs`. Each restart from the table starts
/// the wait again.
pub(super) async fn run(
    tx: Sender<Request>,
    mut restart: watch::Receiver<u64>,
    keepalive_secs: u64,
) {
    loop {
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(keepalive_secs)) => {
                if tx.send(Request::Wake).is_err() {
                    break;
                }
            }
            changed = restart.changed() => {
                if changed.is_err() {
                    break;
                }
            }
        }
    }
}

pub(super) fn now_stamp() -> String {
    format_utc(Utc::now())
}

fn elapsed_secs(last_round_at: &str, now: DateTime<Utc>) -> Result<u64, Reason> {
    let last = parse_utc(last_round_at)?;
    Ok(u64::try_from((now - last).num_seconds()).unwrap_or(0))
}

fn parse_utc(text: &str) -> Result<DateTime<Utc>, Reason> {
    if text.len() != 20 {
        return Err(Reason::Schema);
    }
    NaiveDateTime::parse_from_str(text, STAMP_FORMAT)
        .map(|stamp| stamp.and_utc())
        .map_err(|_| Reason::Schema)
}

fn format_utc(now: DateTime<Utc>) -> String {
    now.format(STAMP_FORMAT).to_string()
}

/// Plays the catch-up rounds when the last round is an interval old.
/// Returns whether `world_time` advanced.
pub(super) fn catch_up_if_due(world: &mut World) -> Result<bool, Reason> {
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
