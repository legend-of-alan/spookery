@v01
Feature: Seats
  Proof: v01_play_as_mira_is_schema_and_writes_nothing, v01_p03_player_hello_opens_one_session, v01_p01_second_player_is_session_full, v01_p02_p04_gm_connect_does_not_write_presence_and_a_second_is_full, v01_p15_bad_hellos_are_schema
  One player hello opens one session. A second player or GM is SESSION_FULL. play as player:mira and a bad hello are SCHEMA.

  Scenario: play as player:mira is SCHEMA and writes nothing
    Given a listening shard of packs/oneshot-docks
    When play starts as player:mira
    Then that client exits 1
    And that client's stderr is SCHEMA
    And that client's stdout is empty
    And the pack bytes are unchanged
    And presence humans is 0
    And the listening shard stops

  Scenario: Player hello opens one session
    Given a listening shard of packs/oneshot-docks
    When play starts
    Then the player stdout contains "kind":"perceive"
    And the player stdout contains {"kind":"resume"
    And the player stderr is empty
    And presence humans is 1
    And actors present are pc_rowan
    And state_rev is 1
    And tick_index is 0
    And world_time is 1080
    And there is no tick file
    And session.jsonl does not exist
    And the listening shard stops

  Scenario: Second player connect is SESSION_FULL
    Given a listening shard of packs/oneshot-docks
    When play starts
    Then the player stdout contains "kind":"perceive"
    And presence humans is 1
    When the seated now.yaml is recorded
    And a raw hello player:mira is refused
    Then the refusal line is {"kind":"refused","code":"SESSION_FULL"}
    And the recorded now.yaml bytes are unchanged
    When play starts as player:pc_rowan
    Then that client exits 1
    And that client's stderr is SESSION_FULL
    And that client's stdout is empty
    And the recorded now.yaml bytes are unchanged
    And presence humans is 1
    And the listening shard stops

  Scenario: GM connect does not write presence and a second is SESSION_FULL
    Given a listening shard of packs/oneshot-docks
    When gm starts
    And a raw second gm is refused
    Then the refusal line is {"kind":"refused","code":"SESSION_FULL"}
    And the recorded now.yaml bytes are unchanged
    And presence humans is 0
    When a second gm client starts
    Then that client exits 1
    And that client's stderr is NOT_IN_SPEC
    And that client's stdout is empty
    And the recorded now.yaml bytes are unchanged
    And the listening shard stops

  Scenario: Bad hellos are SCHEMA
    Given a listening shard of packs/oneshot-docks
    When each bad hello is refused
    Then every refusal line is {"kind":"refused","code":"SCHEMA"}
    And the recorded now.yaml bytes are unchanged
    And presence humans is 0
    And the listening shard stops
