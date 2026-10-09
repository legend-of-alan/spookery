Feature: A missing item is not queued

  Scenario: An unknown moves key does not load
    Given a docks table
    When the moves file gains an unknown key
    Then the load refusal is SCHEMA

  Scenario: A system audience does not load
    Given a docks table
    When the moves file gains a system audience
    Then the load refusal is SCHEMA

  Scenario: A relationship row does not load
    Given a docks table
    When the moves file gains a relationship row
    Then the load refusal is SCHEMA

  Scenario: A reputation row does not load
    Given a docks table
    When the moves file gains a reputation row
    Then the load refusal is SCHEMA

  Scenario: A rumor row does not load
    Given a docks table
    When the moves file gains a rumor row
    Then the load refusal is SCHEMA

  Scenario: A mark without the knife does not tick
    Given a docks table where mark needs a knife
    When the player marks nell
    Then world_time is 1080
    And no tick file was written
