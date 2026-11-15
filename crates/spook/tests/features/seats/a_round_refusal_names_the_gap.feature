Feature: A round refusal names the gap

  Scenario: A move refused during the round names the vigor it lacks
    Given a docks table where push and shove each cost 1 vigor
    When the GM is seated and play joins
    And play queues "push"
    And play queues "shove"
    And the GM commits the round
    Then play's error shows "You cannot afford vigor. [CANT_AFFORD]" once
    And the round rejects that habit with "CANT_AFFORD vigor"

  Scenario: A locked exit refused during the round keeps its sentence
    Given a docks table where dropping the tonic locks the loft
    When the GM is seated and play joins
    And play queues "drop tonic"
    And play queues "travel loft"
    And the GM commits the round
    Then play's error shows "The loft door is locked. [NO_EXIT]" once

  Scenario: A missing item refused during the round sends no missing field
    Given a docks table with the GM and Harbormouse seated
    When Harbormouse queues "drop tonic"
    And Harbormouse queues "use tonic"
    And the GM commits the round
    Then Harbormouse's refused frame for "MISSING_ITEM" has no missing field

  Scenario: A strike on someone who is out of sight when the round applies is refused
    Given a docks table with the GM and Harbormouse seated
    When Harbormouse queues "travel wheel_shed"
    And Harbormouse queues "strike briar"
    And the GM commits the round
    Then Harbormouse's refused frame for "NOT_IN_SPEC" has no missing field

  Scenario: A strike the GM sends across rooms is refused at once
    Given a docks table with a seated GM
    When the GM sends Harbormouse's strike on bram
    Then the GM's queue is refused "NOT_IN_SPEC"

  Scenario: A budget refusal during the round sends no missing field
    Given a docks table where one effect is over budget and the GM is seated
    When Harbormouse queues "tap"
    And the GM commits the round
    Then Harbormouse's refused frame for "BUDGET" has no missing field

  Scenario: An NPC habit refused for cost sends no player frame
    Given Bram can pay for a habit until Harbormouse spends Bram's vigor
    When Harbormouse saps Bram
    Then the player sees no refused frame
    And the round rejects that habit with "CANT_AFFORD vigor"
