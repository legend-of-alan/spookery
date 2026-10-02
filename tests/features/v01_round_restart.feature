@v01
Feature: Round restart
  Proof: v01_p18_early_wake_after_commit_does_not_tick_and_a_due_wake_does, v01_p20_queued_wait_survives_gm_drop_and_the_next_wait_applies_both
  A commit that advances world_time restarts the sleep, and an early wake does not tick. A queued wait survives a GM drop, and the next wait applies both.

  Scenario: Round restart early wake after commit does not tick
    Given a temp copy of pack packs/oneshot-docks
    Then schedule.rs defines wake and wake_shard without player_held or gm_held
    And turn.rs calls schedule::wake_shard and does not open a shard
    When the round restart shard seats pc_rowan
    And the round restart shard queues a wait on the open shard
    And the round restart shard commits the queue
    Then the committed world_time is 1085
    And the committed tick_index is 1
    And the committed humans is 1
    And the committed last_tick_at is not 2026-01-01T00:00:00Z
    When the round restart player seat closes
    Then round restart humans is 0
    And the round restart queue is empty
    When a round restart wake runs
    Then that round restart wake did not advance
    And world_time is still the committed stamp
    And tick_index is still the committed stamp
    And last_tick_at is still the committed stamp
    And state_rev is one more than the committed stamp
    And the only tick file is tick-0001.jsonl

  Scenario: Round restart due wake applies the queued wait
    Given a temp copy of pack packs/oneshot-docks
    When the round restart shard opens and queues a wait for pc_rowan
    Then last_tick_at is 2026-01-01T00:00:00Z
    When a round restart wake runs
    Then that round restart wake advanced
    And round restart humans is 0
    And tick_index is 12
    And world_time is 1800
    And the round restart queue is empty
    And tick-0001.jsonl player waits are the queued id
    And tick-0002.jsonl player waits are empty
    When a round restart wake runs
    Then that round restart wake did not advance
    And tick_index is 12
    And the round restart queue is empty

  Scenario: Round restart queued wait survives a GM drop and the next wait applies both
    Given a temp copy of pack packs/oneshot-docks
    When the round restart shard is listening
    And a round restart GM client is seated
    And the round restart player connects and perceives
    Then world_time is 1080
    And round restart humans is 1
    When the round restart player sends wait
    Then the round restart player gained a queued line and not a resume
    And the round restart player stderr does not contain NOT_IN_SPEC
    And world_time is 1080
    And the pack has no tick file
    When the round restart player says before the round
    Then the round restart player gained a say line and not a resume
    And world_time is 1080
    And tick_index is 0
    And the pack has no tick file
    And session.jsonl exists
    When the round restart GM closes
    Then the round restart GM exits 0
    And world_time is 1080
    And tick_index is 0
    And round restart humans is 1
    And the pack has no tick file
    When the round restart player sends wait again
    Then the round restart player gained a resume
    And world_time is 1085
    And tick_index is 1
    And round restart humans is 1
    And the only tick file is tick-0001.jsonl
    And tick-0001.jsonl has both queued waits
    And tick-0001.jsonl world_time_before is 1080
    And tick-0001.jsonl world_time_after is 1085
    And tick-0001.jsonl presence_humans is 1
    And the round restart processes stop
