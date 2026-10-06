Feature: Status is the host view

  Scenario: Status prints the host clocks and writes nothing
    Given a docks table
    When the operator runs status
    Then status exits 0
    And the host text lists the clocks
    And the pack bytes are unchanged

  Scenario: After the door note the player resume omits the host trace
    Given a docks table
    When thirteen ticks pass
    And the operator runs status
    Then status exits 0
    And the host text lists the door note
    And the player resume lists the door message
    And the shop lists the oil and the rooms do not
    And the pack bytes are unchanged
