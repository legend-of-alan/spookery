Feature: The host reports the ports it bound

  Scenario: A table started on port 0 reports its game port and its agent door
    Given a docks table
    When the table listens on port 0
    Then the host reports the game port and the agent door

  Scenario: A player can connect to the reported game port
    Given a docks table
    When the table listens on port 0
    And a player connects to the reported game port
    Then the player receives a perceive line

  Scenario: Two tables started on port 0 at the same time get distinct ports
    Given a docks table
    When two tables listen on port 0 at the same time
    Then the tables report distinct ports
