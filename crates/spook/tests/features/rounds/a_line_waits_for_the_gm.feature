Feature: A line waits for the GM

  Scenario: With no GM a wait commits one round
    Given a docks table with a seated player
    When the player waits
    Then world_time is 1085
    And round index is 1
    And a round record was written

  Scenario: An empty commit writes no round file
    Given a docks table with a seated GM
    When the GM commits the round
    Then no round record was written
    And world_time is 1080

  Scenario: spook round plays one round
    Given a docks table
    When the operator runs spook round
    Then the command exits 0
    And round index is 1
    And a round record was written
