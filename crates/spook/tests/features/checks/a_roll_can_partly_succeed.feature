Feature: A roll can partly succeed

  Scenario: A high total applies the success list and the shared effects
    Given a docks table where press grit base is 9
    When the player presses Mira
    Then the player line is queued
    When the GM commits the round
    Then Mira is touched and clean
    And the player sees outcome "success" and no faces
    And the GM sees outcome "success" and the faces

  Scenario: A middle total applies the partial list and the shared effects
    Given a docks table where press grit base is 6
    When the player presses Mira
    Then the player line is queued
    When the GM commits the round
    Then Mira is touched and halfway
    And the player sees outcome "partial" and no faces
    And the GM sees outcome "partial" and the faces
    And replaying the tick reproduces the save

  Scenario: A low total applies the failure list and the shared effects
    Given a docks table where press grit base is 0
    When the player presses Mira
    Then the player line is queued
    When the GM commits the round
    Then Mira is touched and missed
    And the player sees outcome "miss" and no faces
    And the GM sees outcome "miss" and the faces

  Scenario: Play prints the partial sentence
    Given a docks copy where press grit base is 6
    When play joins the table on a free loopback address
    And play types "press mira"
    Then play shows "Your roll partly succeeded."
    And play shows no protocol JSON
    And Mira is touched and halfway

  Scenario: A binary row never produces partial
    Given a docks table with a GM and a seated player
    When the player rolls grit
    Then the player line is queued
    When the GM commits the round
    Then the player sees outcome "success" and no faces
    And the GM sees outcome "success" and the faces
    And the check outcome is not "partial"

  Scenario: An opposed roll on a three-band row is binary
    Given a docks table where Mira opposes a three-band press
    When the GM queues Mira's defense
    And the player presses Mira
    Then the player line is queued
    When the GM commits the round
    Then Mira guarded and the press missed
    And the player sees outcome "miss" and no faces
    And the GM sees a binary opposed press

  Scenario: An NPC roll on a three-band row can be partial
    Given a docks table where Mira can partly notice
    When the GM queues Mira's partial notice
    And the GM commits the round
    Then Mira is touched and halfway
    And the GM sees outcome "partial" and the faces
    And the player sees no check frame
    And replaying the tick reproduces the save

  Scenario Outline: A partial band that does not fit is refused
    Given a docks table
    When the moves file gains "<fault>"
    And the operator runs spook check
    Then the check exits 1
    And the report names "rules/moves.yaml"
    And the report names "SCHEMA"
    And the report says "<phrase>"
    And the report has a location for "rules/moves.yaml"
    And stdout is empty

    Examples:
      | fault                       | phrase                 |
      | partial_at without partial  | without a partial      |
      | partial without partial_at  | without partial_at     |
      | partial_at above difficulty | at or above difficulty |
      | partial_at without a check  | without a check        |
