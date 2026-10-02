@v01
Feature: GM say
  Proof: v01_gm_say_appends_and_session_status_returns_the_tape
  Sitting session_status accepts an empty object or since. Any other key is SCHEMA and reads nothing. The output is the v0 object, then queued, then last_seq. queued is the in-memory intents in queue order, each with id, actor_id, and move. last_seq is the last complete tape seq. When since is present, events are stored tape objects whose seq is greater than since, in the GM say shape.

  Scenario: GM say appends the tape and session_status since returns it
    Given a listening GM say shard
    When the GM says from the chair at the table
    Then the say result precedes the spook event
    And the say body is kind say from gm at the table with text from the chair
    And the say body place and actor are null and recipients are empty
    And the spook event is the same say and has no id
    And session.jsonl contains the gm chair line and no tick
    And now.yaml matches the pre-say bytes
    And the GM say shard is still running
    When the GM says a spoofed from
    Then the spoofed say is SCHEMA
    And session.jsonl still has one line
    And now.yaml matches the pre-say bytes
    When the GM calls session_status with an empty object
    Then that status has no events
    And that status queued is empty and last_seq is 1
    And that status presence and pause are objects
    And that status world_time is 1080 and tick_index is 0
    And that status has no code
    When the GM calls session_status with since 0
    Then that status last_seq is 1 and its one event is the chair say
    And that event has no tick_index or world_time
    When the GM calls session_status with since and an extra key
    Then that status is SCHEMA and has no last_seq
    And session.jsonl is unchanged after the extra key
    When the GM say player connects and perceives
    Then the player hello is one perceive
    When the player calls perceive as player:pc_rowan
    Then the player stderr is NOT_IN_SPEC
    And the player stdout still has one perceive and no jsonrpc
    When the player calls tools list
    Then the player stderr is NOT_IN_SPEC twice
    And the player stdout does not contain session_status
    And now.yaml matches the seated bytes
    When the player says across the table
    And the GM says back at you
    Then the GM say result is from gm
    And the player stdout contains the gm reply back at you
    And now.yaml matches the seated bytes
    When the player sends wait
    And the GM calls session_status with since 0
    Then that status lists the player say and the queued wait
    And now.yaml matches the seated bytes
    And the GM say pack has no tick file
    And the GM say session ends
