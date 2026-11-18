Feature: A roll can partly succeed

  Scenario: A high total applies the success list and the shared effects
    Given a docks table where press vigor base is 9
    When the player presses Mira
    Then the player line is queued
    When the GM commits the round
    Then Mira is touched and clean
    And the player sees outcome "success" and no faces
    And the GM sees outcome "success" and the faces

  Scenario: A middle total applies the partial list and the shared effects
    Given a docks table where press vigor base is 6
    When the player presses Mira
    Then the player line is queued
    When the GM commits the round
    Then Mira is touched and halfway
    And the player sees outcome "partial" and no faces
    And the GM sees outcome "partial" and the faces
    And spook verify passes

  Scenario: A low total applies the failure list and the shared effects
    Given a docks table where press vigor base is 0
    When the player presses Mira
    Then the player line is queued
    When the GM commits the round
    Then Mira is touched and missed
    And the player sees outcome "miss" and no faces
    And the GM sees outcome "miss" and the faces

  Scenario Outline: Play prints the roll's sentence
    Given a docks copy where press vigor base is <base>
    When play joins the table on a free loopback address
    And play types "press mira"
    Then play shows "<sentence>"

    Examples:
      | base | sentence                    |
      | 6    | Your roll partly succeeded. |
      | 0    | Your roll missed.           |

  Scenario: A binary row never produces partial
    Given a docks table with a GM and a seated player
    When the player rolls exert
    Then the player line is queued
    When the GM commits the round
    Then the player sees outcome "success" and no faces
    And the GM sees outcome "success" and the faces
    And the check outcome is not "partial"

  Scenario: An NPC roll on a three-band row can be partial
    Given a docks table where Mira can partly notice
    When the GM queues Mira's partial notice
    And the GM commits the round
    Then Mira is touched and halfway
    And the GM sees outcome "partial" and the faces
    And the player sees no check frame
    And spook verify passes
