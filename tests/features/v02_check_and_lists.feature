@v02
Feature: Check and lists
  Proof: v02_p10_a_row_names_a_check_and_the_templates_run_either_way and v02_p11_a_success_list_and_a_failure_list
  One effects list runs on both outcomes. A success list and a failure list each run on their own outcome.

  Scenario: A row names a check and the templates run either way
    Given the lists port is locked
    Given a lists check pack labeled "check-pass" with grit 1
    Then the lists mark row names check grit
    When a lists player sits with no GM
    Then lists humans is 1
    And lists world_time is 1080
    And lists tick_index is 0
    And lists Rowan is in docks_shop
    And lists actor mira has no flags
    And lists tick files are none
    When the lists player marks mira
    Then the lists player sees check success true with a resume and no faces
    And the lists player stderr is empty
    And lists humans is 1
    And lists world_time is 1085
    And lists tick_index is 1
    And lists Rowan is in docks_shop
    And lists actor mira flags are marked
    And lists actor pc_rowan has no flags
    And lists tick "tick-0001.jsonl" world_time goes from 1080 to 1085
    And lists tick "tick-0001.jsonl" traces resolve_check
    And lists tick "tick-0001.jsonl" has one player mark intent targeting mira
    And lists tick "tick-0001.jsonl" has one flag effect on mira key marked
    And lists tick "tick-0001.jsonl" derives neither mark nor wave
    Given a lists check pack labeled "check-fail" with grit -20
    Then the lists mark row names check grit
    When a lists player sits with no GM
    Then lists humans is 1
    And lists world_time is 1080
    And lists tick_index is 0
    And lists Rowan is in docks_shop
    And lists actor mira has no flags
    And lists tick files are none
    When the lists player marks mira
    Then the lists player sees check success false with a resume and no faces
    And the lists player stderr is empty
    And lists humans is 1
    And lists world_time is 1085
    And lists tick_index is 1
    And lists Rowan is in docks_shop
    And lists actor mira flags are marked
    And lists actor pc_rowan has no flags
    And lists tick "tick-0001.jsonl" world_time goes from 1080 to 1085
    And lists tick "tick-0001.jsonl" traces resolve_check
    And lists tick "tick-0001.jsonl" has one player mark intent targeting mira
    And lists tick "tick-0001.jsonl" has one flag effect on mira key marked
    And lists tick "tick-0001.jsonl" derives neither mark nor wave
    Then the lists session is closed

  Scenario: A success list and a failure list
    Given the lists attack row loads as token actor, on named, check grit, empty effects, success hp -1, and failure hp 0
    Given the lists port is locked
    Given a lists outcome pack labeled "lists-pass" with grit 1
    Then the lists mark row names check grit with empty effects, success flag marked, and failure flag missed
    When a lists player sits with no GM
    Then lists humans is 1
    And lists world_time is 1080
    And lists tick_index is 0
    And lists Rowan is in docks_shop
    And lists actor mira has no flags
    And lists tick files are none
    When the lists player marks mira
    Then the lists player sees check success true with a resume and no faces
    And the lists player stderr is empty
    And lists humans is 1
    And lists world_time is 1085
    And lists tick_index is 1
    And lists Rowan is in docks_shop
    And lists actor mira flags are marked
    And lists actor pc_rowan has no flags
    And lists tick "tick-0001.jsonl" world_time goes from 1080 to 1085
    And lists tick "tick-0001.jsonl" traces resolve_check
    And lists tick "tick-0001.jsonl" has one player mark intent targeting mira
    And lists tick "tick-0001.jsonl" has one flag effect on mira key marked
    And lists tick "tick-0001.jsonl" derives neither mark nor wave
    Given a lists outcome pack labeled "lists-fail" with grit -20
    Then the lists mark row names check grit with empty effects, success flag marked, and failure flag missed
    When a lists player sits with no GM
    Then lists humans is 1
    And lists world_time is 1080
    And lists tick_index is 0
    And lists Rowan is in docks_shop
    And lists actor mira has no flags
    And lists tick files are none
    When the lists player marks mira
    Then the lists player sees check success false with a resume and no faces
    And the lists player stderr is empty
    And lists humans is 1
    And lists world_time is 1085
    And lists tick_index is 1
    And lists Rowan is in docks_shop
    And lists actor mira flags are missed
    And lists actor pc_rowan has no flags
    And lists tick "tick-0001.jsonl" world_time goes from 1080 to 1085
    And lists tick "tick-0001.jsonl" traces resolve_check
    And lists tick "tick-0001.jsonl" has one player mark intent targeting mira
    And lists tick "tick-0001.jsonl" has one flag effect on mira key missed
    And lists tick "tick-0001.jsonl" derives neither mark nor wave
    Then the lists session is closed
