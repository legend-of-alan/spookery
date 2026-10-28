Feature: Status is the host view

  Scenario: Status prints the host clocks and writes nothing
    Given a docks table
    When the operator runs status
    Then status exits 0
    And the host text lists the clocks
    And the pack bytes are unchanged

  Scenario: After the door note the player resume omits the host trace
    Given a docks table
    When thirteen rounds pass
    And the operator runs status
    Then status exits 0
    And the host text lists the door note
    And the player resume lists the door message
    And the shop lists the oil and the rooms do not
    And the pack bytes are unchanged

  Scenario: The host prints each round's effects once
    Given a docks table with "pc_rowan" seated and no GM
    When "pc_rowan" waits three times
    And the operator runs status
    Then the host printed the door note exactly once
    And every host printout ends with the clocks
    And the status lists the door note

  Scenario: A seat change prints the clocks and no effects
    Given a docks table with "pc_rowan" seated and no GM
    And "pc_rowan" has waited once
    When "pc_rowan" closes the seat
    Then the host's last printout lists the clocks and no effect rows
