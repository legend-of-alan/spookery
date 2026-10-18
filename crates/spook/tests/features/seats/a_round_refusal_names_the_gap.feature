Feature: A round refusal names the gap

  Scenario: A move refused during the round names the grit
    Given a sample table where push and shove each cost 1 grit
    When the GM is seated and play joins
    And play queues "push"
    And play queues "shove"
    And the GM commits the round
    Then play's error shows "You cannot afford grit. [CANT_AFFORD]" once
    And the round rejects that habit with "CANT_AFFORD grit"
    When the operator runs audit
    Then the command exits 0
    And stderr is empty

  Scenario: A locked exit refused during the round keeps its sentence
    Given a sample table where dropping the tonic locks the loft
    When the GM is seated and play joins
    And play queues "drop tonic"
    And play queues "travel loft"
    And the GM commits the round
    Then play's error shows "The loft door is locked. [NO_EXIT]" once

  Scenario: A missing item refused during the round sends no missing field
    Given a sample table with the GM seated
    When Lark queues "take stick"
    And the GM commits the round
    Then Lark's refused frame for "MISSING_ITEM" has no missing field

  Scenario: A budget refusal during the round sends no missing field
    Given a sample table where one effect is over budget and the GM is seated
    When Lark queues "tap"
    And the GM commits the round
    Then Lark's refused frame for "BUDGET" has no missing field

  Scenario: An NPC habit refused for cost sends no player frame
    Given Oden can pay for a habit until Lark spends his grit
    When Lark saps Oden
    Then the player sees no refused frame
    And the round rejects that habit with "CANT_AFFORD grit"
