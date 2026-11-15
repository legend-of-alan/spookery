Feature: A place with no facts still plays

  Scenario: Harbormouse walks into the Loft and back
    Given a docks table
    When Harbormouse plays "travel loft"
    And Harbormouse plays "travel gate_yard"
    Then the journal commits two location effects for Harbormouse
    And harbormouse is in "Gate Yard"

  Scenario: Harbormouse waits in the Gate Yard
    Given a docks table
    When Harbormouse plays "wait"
    Then the wait commits with nothing rejected

  Scenario: A docks NPC habit fires in a place with no facts
    Given Mira stands in a place with no facts
    When a docks round plays
    Then Mira's habit fires
