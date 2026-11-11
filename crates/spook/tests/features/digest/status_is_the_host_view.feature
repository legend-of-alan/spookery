Feature: Status is the host view

  Scenario: Status prints the host meters and writes nothing
    Given a docks table
    When the operator runs status
    Then status exits 0
    And the host text lists the meters
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
    Given a docks table with "rowan" seated and no GM
    When "rowan" waits three times
    And the operator runs status
    Then the host printed the door note exactly once
    And every host printout ends with the meters
    And the status lists the door note

  Scenario: A seat change prints the meters and no effects
    Given a docks table with "rowan" seated and no GM
    And "rowan" has waited once
    When "rowan" closes the seat
    Then the host's last printout lists the meters and no effect rows
