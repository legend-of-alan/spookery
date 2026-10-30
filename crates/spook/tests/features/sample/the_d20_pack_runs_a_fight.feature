Feature: The d20 pack runs a fight

  Scenario: Check reports no problems for the sample pack
    Given a copy of the sample-d20 pack
    When the operator runs spook check --json
    Then the check exits 0
    And stdout says "[]"

  Scenario: A fixed seed succeeds and lowers the raider's hp by the hit amount
    Given a sample-d20 table
    When the player waits 2 and strikes the raider
    Then the roll is "success"
    And the raider's hp is 5

  Scenario: A fixed seed grazes and applies the partial amount
    Given a sample-d20 table
    When the player strikes the raider
    Then the roll is "partial"
    And the raider's hp is 7

  Scenario: Lark at might 1 misses and the raider's hp stays
    Given a sample-d20 table where Lark's might is 1
    When the player strikes the raider
    Then the roll is "miss"
    And the raider's hp is 8

  Scenario: Drinking a tonic raises hp
    Given a sample-d20 table
    When the player drinks
    Then Lark's hp is 12

  Scenario: Drinking without a tonic is refused
    Given a sample-d20 table where Lark has no tonic
    When the player drinks
    Then lark is refused with NOT_IN_SPEC
    And save.yaml is unchanged since the seat
    And no round record was written

  Scenario: A short fight through play drops the raider to zero
    Given a copy of the sample-d20 pack
    When play joins the sample table
    And the save before the fight is kept
    And play types "strike npc_briar"
    Then play shows "Your roll partly succeeded."
    When play types "strike npc_briar"
    And play types "strike npc_briar"
    And play types "strike npc_briar"
    And play types "strike npc_briar"
    Then play shows 2 successful rolls
    And the raider's hp is 0
    And replaying the round reproduces the save
