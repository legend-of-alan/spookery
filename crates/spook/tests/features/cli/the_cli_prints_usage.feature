Feature: The CLI prints usage

  Scenario: spook --help prints usage
    When the operator asks spook for help
    Then the command exits 0
    And stdout shows usage
    And stdout lists the "round" command
    And stdout lists no "tick" command
    And stderr is empty

  Scenario Outline: A command's help prints usage
    When the operator asks the "<command>" command for help
    Then the command exits 0
    And stdout shows usage
    And stderr is empty

    Examples:
      | command |
      | run     |
      | play    |
      | gm      |
      | ai-gm   |
      | round   |
      | status  |
      | migrate |
      | check   |
      | new     |
      | verify  |

  Scenario: A mistyped command is a usage error
    When the operator runs the arguments "rnu" and "x"
    Then the command exits 2
    And stderr shows usage
    And stdout is empty

  Scenario: The old tick command is a usage error
    When the operator runs the arguments "tick" and "x"
    Then the command exits 2
    And stderr shows usage
    And stdout is empty

  Scenario: Play reaches the first perceive on the given address
    Given a docks table
    When play joins the table on a free loopback address
    Then play received the first perceive
