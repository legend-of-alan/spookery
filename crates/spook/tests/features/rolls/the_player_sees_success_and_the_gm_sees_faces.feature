Feature: The player sees success and the GM sees faces

  Scenario: A live exert returns success without faces
    Given a docks table with a seated player
    When the player rolls exert
    Then the player sees a check success
    And world_time is 1085

  Scenario: A held exert shows faces only to the GM
    Given a docks table with a GM and a seated player
    When the player rolls exert
    Then the player line is queued
    And world_time is 1080
    And no round record was written
    When the GM commits the round
    Then the GM sees the exert faces
    And the player sees a check result without faces
