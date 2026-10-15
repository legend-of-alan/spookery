Feature: Speech appends to the tape and does not tick

  Scenario: A seated player says a line to the table
    Given a docks table with a seated player
    When the player says "hello"
    Then the tape contains the text "hello"
    And the tape address is "table"
    And save.yaml is unchanged since the seat
    And state_rev is unchanged
    And no tick file was written

  Scenario: A seated player says a line to the room
    Given a docks table with a seated player
    When the player says "place the lamp is lit"
    Then the tape contains the text "the lamp is lit"
    And the tape address is "place"
    And the tape names the place "docks_shop"
    And the tape recipients are "pc_rowan"
    And save.yaml is unchanged since the seat
    And no tick file was written

  Scenario: A seated player says a line to mira
    Given a docks table with a seated player
    When the player says "mira good evening"
    Then the tape contains the text "good evening"
    And the tape address is "actor"
    And the tape names the actor "mira"
    And save.yaml is unchanged since the seat
    And no tick file was written

  Scenario: A sentence that does not name an actor stays at the table
    Given a docks table with a seated player
    When the player says "good evening mira"
    Then the tape contains the text "good evening mira"
    And the tape address is "table"
    And save.yaml is unchanged since the seat
    And no tick file was written

  Scenario: Say to mira while nobody is seated
    Given a docks table
    When say to mira is submitted on an empty table saying "good evening"
    Then the tape contains the text "good evening"
    And the tape address is "actor"
    And the tape names the actor "mira"
    And save.yaml matches the bytes from before the line
    And no tick file was written
