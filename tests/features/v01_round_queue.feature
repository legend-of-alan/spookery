@v01
Feature: Round queue
  Proof: v01_p10_no_gm_wait_commits_one_live_tick, v01_p10_p11_wake_while_a_seat_is_held_does_not_tick_or_drain, v01_p11_gm_commit_advances_world_time_and_a_held_wake_does_not
  No GM seated: a player wait commits one live tick. humans is 1, so live minutes, not the idle hour.
  A scheduler wake during that seat does not call run_tick.
  GM seated: a player wait only queues. world_time moves on the GM commit_turn and not on the player line.
  A scheduler wake while that GM connection is held does not drain the queue and does not advance world_time.

  Scenario: Round queue no-GM wait commits one live tick
    Given a temp copy of pack packs/oneshot-docks
    When the round queue shard is listening with no GM
    And the round queue player connects and perceives
    Then presence humans is 1
    And world_time is 1080
    And tick_index is 0
    And state/log holds 0 tick jsonl files
    When the round queue player sends wait
    Then the player stdout gained a resume and not a queued line
    And the player stderr does not contain NOT_IN_SPEC
    And presence humans is 1
    And world_time is 1085
    And world_time is not 1140
    And tick_index is 1
    And state_rev increased by 1 from the seated snapshot
    And last_tick_at changed from the seated snapshot
    And the only tick file is tick-0001.jsonl
    And tick-0001.jsonl presence_humans is 1
    And tick-0001.jsonl world_time_before is 1080
    And tick-0001.jsonl world_time_after is 1085

  Scenario: Round queue held wake does not tick or drain
    Given a temp copy of pack packs/oneshot-docks
    When the round queue shard seats pc_rowan
    And a scheduled wake runs with the player held and the GM free
    Then that wake did not advance world_time
    And presence humans is 1
    And world_time is 1080
    And tick_index is 0
    And state/log holds 0 tick jsonl files
    When the round queue shard queues a wait for pc_rowan
    Then the shard queue holds 1 intent
    When a scheduled wake runs with the player held and the GM held
    Then that wake did not advance world_time
    And the shard queue still holds that wait
    And world_time is 1080
    And state/log holds 0 tick jsonl files
    When the round queue player seat closes
    Then presence humans is 0
    And world_time is 1080
    When a scheduled wake runs with the player free and the GM held
    Then that wake did not advance world_time
    And the shard queue holds 1 intent
    And world_time is 1080
    And tick_index is 0
    And state/log holds 0 tick jsonl files

  Scenario: Round queue GM commit advances world_time
    Given a temp copy of pack packs/oneshot-docks
    When the round queue shard is listening with a GM seat
    And the round queue player connects and perceives
    Then presence humans is 1
    And world_time is 1080
    When the GM commits an empty turn
    Then the empty commit log_path is null and the diff is empty
    And world_time is 1080
    And state_rev is still the seated snapshot
    And state/log holds 0 tick jsonl files
    When the round queue player sends wait while a GM is seated
    Then the player stdout gained a queued line and not a resume
    And world_time is 1080
    And state/log holds 0 tick jsonl files
    When the GM commits the queued turn
    Then the commit has no code and log_path ends with tick-0001.jsonl
    And world_time is 1085
    And world_time is not 1140
    And tick_index is 1
    And presence humans is 1
    And state_rev increased by 1 from the seated snapshot
    And last_tick_at changed from the seated snapshot
    And tick-0001.jsonl player waits are the queued id
    And tick-0001.jsonl presence_humans is 1
    And tick-0001.jsonl world_time_after is 1085
