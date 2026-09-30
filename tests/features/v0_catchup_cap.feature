@v0
Feature: Catch-up cap
  Proof: v0_t04_catchup_cap, v0_trace_d_catchup_jsonl
  Trace D on shard ex from world_time 0: 72h downtime commits exactly 12 ticks, one season_advance proposal, tick_index 12, and world_time 720.

  Scenario: Shard ex catch-up after 72 hours commits exactly twelve ticks
    Given shard ex from world_time 0
    When catch-up runs for 72 hours of downtime
    Then state_rev is 12
    And tick_index is 12
    And world_time is 720
    And last_tick_at is 2026-01-01T00:00:00Z
    And proposals are season_advance
    And state/log holds 12 tick jsonl files
    And state/proposals holds season_advance.yaml
    And proposal season_advance.yaml equals the recorded season_advance document
    And world/places/gate.md is unchanged
    And cast/pc_rowan.md is unchanged

  Scenario: Shard ex catch-up writes the Trace D jsonl lines
    Given shard ex from world_time 0
    When catch-up runs for 72 hours of downtime
    Then tick jsonl lines 1 through 11 are empty idle lines for shard ex
    And tick jsonl 12 is the season_advance catch-up line
