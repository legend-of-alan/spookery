Feature: Seat presence stays in memory

  Scenario: Seating a player writes nothing under state
    Given a docks table
    When the state directory is remembered
    And a player sits
    Then the state directory is unchanged
    And state_rev is 0

  Scenario: A player leaving writes nothing under state
    Given a docks table with a seated player
    When the state directory is remembered
    And the player leaves
    Then the state directory is unchanged
    And state_rev is 0

  Scenario: A host restart starts with nobody present
    Given a docks table with a seated player
    When the host restarts
    Then nobody is present

  Scenario: A tick played with one human replays with the same now_hash
    Given a docks table with a seated player
    When the player waits
    And the operator runs verify
    Then the command exits 0
    And the tick now_hash matches the save
    And stderr is empty

  Scenario: Two players seated and dropped one after another get different session ids
    Given a docks table
    When two players sit, one drops, and that PC sits again
    Then the session ids differ
