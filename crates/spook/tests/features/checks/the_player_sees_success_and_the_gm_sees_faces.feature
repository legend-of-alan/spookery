Feature: The player sees success and the GM sees faces

  Scenario: A live grit returns success without faces
    Given a docks table with a seated player
    When the player rolls grit
    Then the player sees a check success
    And the round log resolves the check without faces
    And world_time is 1085

  Scenario: A held grit shows faces only to the GM
    Given a docks table with a GM and a seated player
    When the player rolls grit
    Then the player line is queued
    And world_time is 1080
    And no round file was written
    When the GM commits the round
    Then the GM sees the grit faces
    And the player sees a check result without faces
    And the round log resolves the check without faces
