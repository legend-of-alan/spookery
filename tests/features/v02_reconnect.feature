@v02
Feature: Reconnect
  Proof: v02_p04_reconnect_returns_scrollback_and_keeps_the_queue_once
  The same PC drops and connects again. That connect is a new session_open for the same actor, as v0.1.
  Scrollback returns. humans is 1. Intents already queued are still queued, once each.
  Reconnect does not apply them. No new session-id rule. No durable queue.

  Scenario: Reconnect returns scrollback and keeps the queue once
    Given the reconnect port is locked
    When a reconnect player sits with a GM on shard "b-again"
    Then reconnect humans is 1
    And reconnect actors present are pc_rowan
    And reconnect world_time is 1080
    And reconnect tick_index is 0
    When the reconnect player sends "say the tin is out"
    Then the reconnect player stdout contains "the tin is out"
    And reconnect world_time is 1080
    And reconnect tick files are none
    When the reconnect player sends "wait"
    Then reconnect queued lines are 1
    When the reconnect player sends "travel mira_rooms"
    Then reconnect queued lines are 2
    And reconnect world_time is 1080
    And reconnect tick_index is 0
    And reconnect Rowan is in docks_shop
    And reconnect tick files are none
    And the reconnect GM status shows humans 1 and the stored queue
    And reconnect now.yaml bytes contain neither queued id
    When the reconnect player sends "close"
    Then the reconnect player exits 0
    And reconnect humans becomes 0
    And reconnect world_time is 1080
    And reconnect tick_index is 0
    And reconnect actors present are empty
    And reconnect tick files are none
    And the reconnect GM status shows humans 0 and the stored queue
    When the reconnect player sits again
    Then reconnect scrollback has one say of "the tin is out"
    And reconnect humans is 1
    And reconnect actors present are pc_rowan
    And reconnect world_time is 1080
    And reconnect tick_index is 0
    And reconnect Rowan is in docks_shop
    And reconnect tick files are none
    And reconnect session tape contains "the tin is out" once
    And the reconnect GM status shows humans 1 and the stored queue
    When the reconnect GM commits the turn including a tick
    Then the reconnect commit has no code and log_path ends with "tick-0001.jsonl"
    And reconnect world_time is 1085
    And reconnect world_time is not 1140
    And reconnect tick_index is 1
    And reconnect humans is 1
    And reconnect Rowan is in docks_shop
    And reconnect tick files are "tick-0001.jsonl"
    And reconnect tick "tick-0001.jsonl" records humans 1 from 1080 to 1085
    And reconnect tick "tick-0001.jsonl" applies the wait id once
    And reconnect tick "tick-0001.jsonl" rejects the travel id as OUT_OF_RANGE
    And reconnect tick "tick-0001.jsonl" keeps the travel id out of intents
    And reconnect tick "tick-0001.jsonl" closes effects on the travel id
    And the reconnect GM queue is empty
    When the reconnect GM commits the turn including a tick again
    Then the reconnect commit has no code and log_path is null
    And reconnect world_time is 1085
    And reconnect tick_index is 1
    And reconnect humans is 1
    And reconnect Rowan is in docks_shop
    And reconnect tick files are "tick-0001.jsonl"
    And the reconnect GM queue is empty
