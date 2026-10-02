@v02
Feature: Unknown flag
  Proof: v02_p08_an_unlisted_flag_is_unknown_and_derivation_skips_the_row
  A key that is not listed returns UNKNOWN_KEY and writes nothing. Derivation skips the row.

  Scenario: An unlisted flag is UNKNOWN_KEY and derivation skips the row
    Given an action pack with mark and wave flag rows and a seated player and GM
    Then flag world_time is 1080
    And flag actor mira has no flags
    And flag actor aldric has no flags
    And flag actor pc_rowan has no flags
    When the flag player line "wave extra" is refused as SCHEMA
    And the flag player queues "wave"
    Then flag world_time is 1080
    And the flag tick log is empty
    And flag actor pc_rowan has no flags
    And flag actor mira has no flags
    And session_status queues the wave row
    When the flag GM commits the turn with a tick
    Then the flag commit has no code
    And flag world_time is 1085
    And flag actor pc_rowan has no flags
    And flag actor mira has no flags
    And flag actor aldric has no flags
    And flag tick "tick-0001.jsonl" rejects the wave id as UNKNOWN_KEY
    And flag tick "tick-0001.jsonl" has no flag effects
    And flag tick "tick-0001.jsonl" intents are not wave or mark
    And flag tick "tick-0001.jsonl" derives neither mark nor wave
    When the flag player queues "mark aldric"
    Then flag world_time is 1085
    And flag actor aldric has no flags
    When the flag GM commits the turn with a tick
    Then the flag commit has no code
    And flag world_time is 1090
    And flag tick_index is 2
    And flag actor aldric has no flags
    And flag actor mira has no flags
    And flag actor pc_rowan has no flags
    And flag actor bram has no flags
    And flag tick "tick-0002.jsonl" rejects the mark id as UNKNOWN_KEY
    And flag tick "tick-0002.jsonl" has no flag effects
    And flag tick "tick-0002.jsonl" effects do not name the mark id
    And flag tick "tick-0002.jsonl" intents are not the mark row
    And flag tick "tick-0002.jsonl" derives neither mark nor wave
    And the flag session ends
