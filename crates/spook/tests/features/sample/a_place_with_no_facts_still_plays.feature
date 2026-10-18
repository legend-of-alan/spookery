Feature: A place with no facts still plays

  Scenario: Lark walks into the Loft and back
    Given a copy of the sample-d20 pack
    When Lark plays "travel loft"
    And Lark plays "travel gate_yard"
    Then the journal commits two location effects for Lark
    And Lark is in the Gate Yard
    And no round writes empty_perceive

  Scenario: Nell waits on Mill Lane
    Given a copy of the sample-d20 pack
    When Nell waits
    Then the wait commits with nothing rejected
    And no round writes empty_perceive

  Scenario: A docks NPC habit fires in a place with no facts
    Given Mira stands in a place with no facts
    When a docks round plays
    Then Mira's habit fires
    And no round writes empty_perceive
