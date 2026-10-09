Feature: Only loopback binds

  Scenario: An open address is refused
    Given a docks table
    When the operator runs run bound to "0.0.0.0"
    Then the command exits 1
    And stderr is NOT_IN_SPEC
    And stdout is empty
    And the pack bytes are unchanged

  Scenario Outline: Every command refuses an open address before the pack opens
    Given a docks table
    When the operator runs <command> bound to "<addr>"
    Then the command exits 1
    And stderr is NOT_IN_SPEC
    And stdout is empty
    And the pack bytes are unchanged

    Examples:
      | command | addr          |
      | play    | 0.0.0.0       |
      | ai-gm   | 0.0.0.0       |
      | tick    | 0.0.0.0       |
      | tick    | 10.0.0.1:9737 |
      | status  | 0.0.0.0       |
      | status  | 10.0.0.1:9737 |
      | migrate | 0.0.0.0       |
      | migrate | 10.0.0.1:9737 |

  Scenario: Run help names the default loopback address
    When the operator asks the "run" command for help
    Then the command exits 0
    And stdout shows usage
    And stdout names the default loopback address

  Scenario: An explicit loopback port accepts a connection
    Given a docks table
    When the table listens on a free loopback port
    Then a connection to the table succeeds
    And the listen port is not 9737
