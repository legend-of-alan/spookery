@v02
Feature: Travel queue
  Proof: v02_p03_travel_queues_until_commit_or_commits_one_live_tick
  travel <place> is one place id, on the same queue path as wait.
  With a GM seated, the line returns queued, does not advance world_time, and does not move the actor until commit_turn.
  With no GM, it commits one live tick.
  A missing place is SCHEMA and queues nothing.
  Any other shape of a travel line is SCHEMA and queues nothing.
  The applying tick uses the location effect expand already returns.
  A place the pack does not have stays that tick's existing kernel refusal.
  No new effect class. NPC derivation is unchanged.
  attack, give, and take stay NOT_IN_SPEC and queue nothing.

  Scenario: Travel queues until commit or commits one live tick
    Given the travel port is locked
    When a travel player sits on shard "b-wait"
    Then travel humans is 1
    And travel world_time is 1080
    And travel Rowan is in docks_shop
    When the travel player sends "wait"
    Then the travel player stdout gains one resume
    And travel world_time is 1085
    And travel humans is 1
    And travel Rowan is in docks_shop
    And travel tick files are "tick-0001.jsonl"
    And travel tick "tick-0001.jsonl" includes a player wait intent
    And that tick is the travel derivation baseline
    When a travel player sits with a GM on shard "b-seated"
    Then travel humans is 1
    And travel world_time is 1080
    And travel Rowan is in docks_shop
    And travel tick files are none
    And the travel opened clock is stored
    When the travel player line "travel" is refused SCHEMA
    And the travel player line "travel mira_rooms extra" is refused SCHEMA
    And the travel player line "travel  mira_rooms" is refused SCHEMA
    And the travel player line " wait" is refused SCHEMA
    And the travel player line "attack" is refused NOT_IN_SPEC
    And the travel player line "attack mira" is refused NOT_IN_SPEC
    And the travel player line "give" is refused NOT_IN_SPEC
    And the travel player line "take" is refused NOT_IN_SPEC
    And the travel player line "dance" is refused NOT_IN_SPEC
    Then travel world_time is 1080
    And travel tick_index is 0
    And travel Rowan is in docks_shop
    And travel tick files are none
    And the travel GM queue is empty
    When the travel player sends "travel mira_rooms"
    Then the travel player stdout gains one queued travel id and no resume or refusal
    And travel world_time is 1080
    And travel tick_index is 0
    And travel Rowan is in docks_shop
    And travel tick files are none
    And the travel GM queue is the travel id moving travel
    And the travel now.yaml bytes do not contain the travel id
    When the travel GM commits the turn including a tick
    Then the travel commit has no code and log_path ends with "tick-0001.jsonl"
    And travel world_time is 1085
    And travel world_time is not 1140
    And travel tick_index is 1
    And travel humans is 1
    And travel Rowan is in docks_shop
    And travel last_tick_at differs from the opened clock
    And travel tick files are "tick-0001.jsonl"
    And travel tick "tick-0001.jsonl" records humans 1 from 1080 to 1085
    And travel tick "tick-0001.jsonl" rejects the travel id as OUT_OF_RANGE
    And travel tick "tick-0001.jsonl" closes effects on the travel id
    And that tick is the travel seated derivation skipping the travel id
    And the travel GM queue is empty
    When the travel player sends "travel not_a_place"
    Then the travel player stdout gains one new queued id stored as the missing id
    And travel world_time is 1085
    And travel Rowan is in docks_shop
    And travel tick files are "tick-0001.jsonl"
    And the travel GM queue is the missing id moving travel
    When the travel GM commits the turn including a tick
    Then the travel second commit has no code
    And travel world_time is 1090
    And travel tick_index is 2
    And travel humans is 1
    And travel Rowan is in docks_shop
    And travel tick files are "tick-0001.jsonl,tick-0002.jsonl"
    And travel tick "tick-0002.jsonl" records time from 1085 to 1090
    And travel tick "tick-0002.jsonl" rejects the missing id as UNKNOWN_KEY
    And travel tick "tick-0002.jsonl" closes effects on the missing id
    And travel tick "tick-0002.jsonl" leaves the travel id out of rejected and intents
    When a travel player sits on shard "b-live"
    Then travel world_time is 1080
    And travel Rowan is in docks_shop
    And the travel live clock is stored
    When the travel player line "travel" is refused SCHEMA
    And the travel player line "travel mira_rooms extra" is refused SCHEMA
    And the travel player line "attack" is refused NOT_IN_SPEC
    Then travel world_time is 1080
    And travel tick_index is 0
    And travel tick files are none
    And travel Rowan is in docks_shop
    When the travel player sends "travel mira_rooms"
    Then the travel player stdout gains one resume and no queued line
    And travel humans is 1
    And travel world_time is 1085
    And travel world_time is not 1140
    And travel tick_index is 1
    And travel last_tick_at differs from the live clock
    And travel Rowan is in docks_shop
    And travel tick files are "tick-0001.jsonl"
    And travel tick "tick-0001.jsonl" records humans 1 from 1080 to 1085
    And travel tick "tick-0001.jsonl" has one new rejection OUT_OF_RANGE beyond the baseline
    And travel tick "tick-0001.jsonl" closes effects on the live travel id
    And that tick is the travel live derivation skipping the live travel id
    Then the travel seated derivation matches the baseline
    And the travel live derivation matches the baseline
    And the travel shard stops
