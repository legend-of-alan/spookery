Feature: Only loopback binds

  Scenario: An open address is refused
    Given a docks table
    When the operator runs run bound to "0.0.0.0"
    Then the command exits 1
    And stderr is NOT_IN_SPEC
    And stdout is empty
    And the pack bytes are unchanged

  Scenario: The default loopback address accepts a connection
    Given a docks table
    When the table listens on 127.0.0.1
    Then a connection to the table succeeds

  Scenario: An explicit loopback port accepts a connection
    Given a docks table
    When the table listens on a free loopback port
    Then a connection to the table succeeds
    And the listen port is not 9737
