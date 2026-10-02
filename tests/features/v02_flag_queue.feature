@v02
Feature: Flag queue
  Proof: v02_p07_mark_queues_until_commit_or_commits_one_live_tick
  A mark row queues until commit_turn when a GM is seated, and commits one live tick when no GM is seated.

  Scenario: Flag queue with a GM seated queues mark until commit
    Given the flag queue port is locked
    And a flag action pack labeled "row-seated"
    When a flag player sits with a GM
    Then flag humans is 1
    And flag world_time is 1080
    And flag Rowan is in docks_shop
    And flag actor mira has no flags
    When the player line "mark" is flag SCHEMA
    And the player line "mark mira extra" is flag SCHEMA
    And the player line "mark  mira" is flag SCHEMA
    And the player line "attack" is flag NOT_IN_SPEC
    And the player line "attack mira" is flag NOT_IN_SPEC
    And the player line "dance" is flag NOT_IN_SPEC
    Then flag world_time is 1080
    And flag tick_index is 0
    And flag tick files are none
    And flag actor mira has no flags
    And flag GM queue is empty
    When the player writes the flag line "mark mira"
    Then the flag player gained one queued id and no resume
    And flag world_time is 1080
    And flag tick_index is 0
    And flag actor mira has no flags
    And flag tick files are none
    And flag GM queue is the saved mark id for move mark
    When the GM flag-commits the turn
    Then the flag commit has no code
    And flag world_time is 1085
    And flag world_time is not 1140
    And flag tick_index is 1
    And flag humans is 1
    And flag actor mira flags are marked
    And flag actor pc_rowan has no flags
    And flag actor aldric has no flags
    And flag actor bram has no flags
    And flag Rowan is in docks_shop
    And flag tick "tick-0001.jsonl" world_time goes from 1080 to 1085
    And flag tick "tick-0001.jsonl" has one player mark intent for the saved mark id targeting mira
    And flag tick "tick-0001.jsonl" has one flag effect for the saved mark id on mira key marked
    And flag tick "tick-0001.jsonl" derives neither mark nor wave
    And flag GM queue is empty
    When the player writes the flag line "mark nobody"
    Then the flag player gained one queued id saved as missing
    And flag world_time is 1085
    And flag actor mira flags are marked
    And flag GM queue is the saved missing id for move mark
    When the GM flag-commits the turn
    Then the flag commit has no code
    And flag world_time is 1090
    And flag tick_index is 2
    And flag actor nobody is absent
    And flag actor mira flags are marked
    And flag tick "tick-0002.jsonl" rejects the saved missing id as UNKNOWN_KEY
    And flag tick "tick-0002.jsonl" has no flag effects
    And flag tick "tick-0002.jsonl" intents omit the saved missing id
    When the player line "travel" is flag SCHEMA
    And the player writes the flag line "travel mira_rooms"
    Then the flag player gained one queued id saved as travel
    And flag world_time is 1090
    And flag Rowan is in docks_shop
    And flag GM queue is the saved travel id for move travel
    When the GM flag-commits the turn
    Then the flag commit has no code
    And flag world_time is 1095
    And flag tick_index is 3
    And flag Rowan is in docks_shop
    And flag actor mira flags are marked
    And flag tick "tick-0003.jsonl" rejects the saved travel id as OUT_OF_RANGE
    And flag tick "tick-0003.jsonl" closes effects for the saved travel id
    And the flag session is closed

  Scenario: Flag queue with no GM commits one live tick
    Given the flag queue port is locked
    And a flag action pack labeled "row-live"
    When a flag player sits with no GM
    Then flag world_time is 1080
    And flag actor mira has no flags
    When the player line "mark" is flag SCHEMA
    And the player line "mark mira extra" is flag SCHEMA
    And the player line "attack" is flag NOT_IN_SPEC
    Then flag world_time is 1080
    And flag tick_index is 0
    And flag tick files are none
    And flag actor mira has no flags
    When the player writes the flag line "mark mira"
    Then the flag player gained a resume and no queued line
    And flag humans is 1
    And flag world_time is 1085
    And flag world_time is not 1140
    And flag tick_index is 1
    And flag actor mira flags are marked
    And flag actor pc_rowan has no flags
    And flag Rowan is in docks_shop
    And flag tick "tick-0001.jsonl" world_time goes from 1080 to 1085
    And flag tick "tick-0001.jsonl" has one flag effect on mira key marked
    And flag tick "tick-0001.jsonl" derives neither mark nor wave
    And the flag session is closed
