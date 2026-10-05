Feature: Speech appends to the tape and does not tick

  Scenario: A seated player says a line
    Given a docks table with a seated player
    When the player says "hello"
    Then the tape contains the text "hello"
    And now.yaml is unchanged since the seat
    And state_rev is unchanged
    And no tick file was written

  Scenario: A seated player talks to an actor
    Given a docks table with a seated player
    When the player talks to mira saying "good evening"
    Then the tape contains the text "good evening"
    And now.yaml is unchanged since the seat
    And state_rev is unchanged
    And no tick file was written
