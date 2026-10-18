Feature: A move refused in its round says why

  Scenario: A move refused in its round says why
    Given a copy of the sample-d20 pack
    When play joins the sample table
    And play types "take stick"
    Then play's error shows "You do not have that. [MISSING_ITEM]"
    When play types "wield tonic"
    Then play's error shows "That is not a weapon. [NOT_A_WEAPON]"
    And play's error shows "[MISSING_ITEM]" once
    And play's error shows "[NOT_A_WEAPON]" once
    And play shows no protocol JSON
    When the operator runs audit
    Then the command exits 0
    And stderr is empty

  Scenario: A refused NPC habit sends no player frame
    Given Oden's habit takes a tonic that is not there
    When the player waits for the resume
    Then the player sees no refused frame
    And the round rejects that habit with MISSING_ITEM
