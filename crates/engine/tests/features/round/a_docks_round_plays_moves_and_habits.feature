Feature: A docks round plays moves and habits

  Scenario: Harbormouse walks into the Loft and back
    Given a docks table
    When Harbormouse plays "travel loft"
    And Harbormouse plays "travel gate_yard"
    Then harbormouse is in "Gate Yard"

  Scenario: Harbormouse waits in the Gate Yard
    Given a docks table
    When Harbormouse plays "wait"
    Then the wait lands with nothing refused

  Scenario: Mira's habit leaves her door note
    Given a docks table
    When a docks round plays
    Then Mira's habit leaves her door note
