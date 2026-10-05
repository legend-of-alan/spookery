Feature: GM without stdio refuses

  Scenario: spook gm without stdio exits NOT_IN_SPEC
    Given a docks table
    When the operator runs gm
    Then the command exits 1
    And stderr is NOT_IN_SPEC
    And stdout is empty
    And the pack bytes are unchanged
