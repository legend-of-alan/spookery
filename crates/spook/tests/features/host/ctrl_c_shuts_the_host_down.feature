Feature: Ctrl+C shuts the host down

  Scenario: SIGINT after a player sits exits 0 and verifies
    Given a docks table
    When spook run seats a player
    And spook run receives SIGINT
    Then spook run exits 0 and spook verify passes

  Scenario: Closed stdin keeps the host serving until a signal
    Given a docks table
    When spook run starts with stdin closed
    And a player sits at that host
    Then spook run is still serving
    When spook run receives SIGINT
    Then spook run exits 0

  Scenario: A second SIGINT ends the process when shutdown blocks
    Given a docks table
    When spook run seats a player and shutdown will block
    And spook run receives SIGINT
    Then shutdown is blocked and spook run is still running
    When spook run receives SIGINT
    Then spook run has ended
