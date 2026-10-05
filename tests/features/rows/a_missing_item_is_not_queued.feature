Feature: A missing item is not queued

  Scenario: An empty item id is refused at load
    Given a docks table
    When the moves file gains an empty item id
    Then the load refusal is SCHEMA

  Scenario: An unknown move key is refused at load
    Given a docks table
    When the moves file gains an unknown key
    Then the load refusal is SCHEMA

  Scenario: A mark without the knife is refused
    Given a docks table where mark needs a knife
    When the player marks nell
    Then the player is refused with MISSING_ITEM
    And world_time is 1080
    And no tick file was written

  Scenario: A mark without the knife is refused while a GM is seated
    Given a docks table where mark needs a knife and a GM is seated
    When the player marks nell
    Then the player is refused with MISSING_ITEM
    And world_time is 1080
    And no tick file was written
