Feature: A PC's verbs play in a docks round

  Scenario: Harbormouse walks into the Loft and back
    Given a docks table
    When Harbormouse plays "travel loft"
    And Harbormouse plays "travel gate_yard"
    Then harbormouse is in "Gate Yard"

  Scenario: Harbormouse waits in the Gate Yard
    Given a docks table
    When Harbormouse plays "wait"
    Then the wait lands with nothing refused
