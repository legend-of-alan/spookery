Feature: A missing item is not queued

  Scenario: An effect kind the engine does not have does not load
    Given a docks table
    When the moves file gains a rumor row
    Then the load refusal is SCHEMA

  Scenario: A mark without the knife does not play a round
    Given a docks table where mark needs a knife
    When the player marks nell
    Then world_time is 1080
    And no round record was written
