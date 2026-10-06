Feature: A line waits for the GM

  Scenario: With no GM a wait commits one tick
    Given a docks table with a seated player
    When the player waits
    Then world_time is 1085
    And tick index is 1
    And a tick file was written

  Scenario: With a GM a wait stays queued until commit
    Given a docks table with a GM and a seated player
    When the player waits
    Then the player line is queued
    And world_time is 1080
    And no tick file was written
    When the GM commits the round
    Then world_time is 1085
    And the tick log records the player wait

  Scenario: An empty commit writes no tick file
    Given a docks table with a seated GM
    When the GM commits the round
    Then no tick file was written
    And world_time is 1080
