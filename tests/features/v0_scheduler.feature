@v0
Feature: v0 scheduler
  Proof: v0_scheduler_sleeps_then_catches_up_on_this_thread
  The scheduler sleeps tick_every_real_seconds on a Tokio timer and calls run_tick on the calling thread.

  Scenario: Scheduler sleeps tick_every_real_seconds then catches up on the calling thread
    Given a scheduler pack with a 1 second tick and last_tick_at 2026-01-01T00:00:00Z
    Then schedule.rs sleeps on a Tokio timer
    And schedule.rs does not spawn a tick
    When the scheduler wakes the pack on this thread
    Then the wake slept at least 1 second and under 20 seconds
    And tick_index is 12
    And state_rev is 12
    And world_time is 720
    And proposals are season_advance
    And last_tick_at is a fresh 20-character UTC stamp
