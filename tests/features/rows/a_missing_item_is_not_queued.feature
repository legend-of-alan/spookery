Feature: A missing item is not queued

  Scenario: An unknown moves key does not load
    Given a docks table
    When the moves file gains an unknown key
    Then the load refusal is SCHEMA

  Scenario: A mark without the knife does not tick
    Given a docks table where mark needs a knife
    When the player marks nell
    Then world_time is 1080
    And no tick file was written
