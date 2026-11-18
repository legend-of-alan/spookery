Feature: The host reports the ports it bound

  Scenario: A table started on port 0 reports its game port, its MCP address and its log file
    Given a docks table
    When the table listens on port 0
    Then the host reports the game port and the MCP address
    And the host names its log file in the directory it started in, and the file says the pack opened
    And the pack holds no log file

  Scenario: Two tables started on port 0 at the same time get distinct ports and log files
    Given a docks table
    When two tables listen on port 0 at the same time
    Then the tables report distinct ports
    And the tables name different log files
