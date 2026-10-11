Feature: The d20 pack runs a fight

  Scenario: Check reports no problems for the sample pack
    Given a copy of the sample-d20 pack
    When the operator runs spook check --json
    Then the check exits 0
    And stdout says "[]"

  Scenario: A sure strike lowers the raider's hp by the hit amount
    Given a sample-d20 table
    When the player strikes the raider
    Then the raider's hp is 4

  Scenario: A weak strike misses and the raider's hp stays
    Given a sample-d20 table where Lark's might is 0
    When the player strikes the raider
    Then the raider's hp is 8

  Scenario: A middling strike grazes and applies the partial amount
    Given a sample-d20 table where Lark's might is 20
    When the player strikes the raider
    Then the raider's hp is 7

  Scenario: Drinking a tonic raises hp
    Given a sample-d20 table
    When the player drinks
    Then Lark's hp is 12

  Scenario: Drinking without a tonic is refused
    Given a sample-d20 table where Lark has no tonic
    When the player drinks
    Then the player was refused MISSING_ITEM
    And now.yaml is unchanged since the seat
    And no tick file was written

  Scenario: A short fight through play drops the raider to zero
    Given a copy of the sample-d20 pack
    When play joins the sample table
    And the save before the fight is kept
    And play types "strike npc_briar"
    Then play shows "Your roll succeeded."
    When play types "strike npc_briar"
    Then play shows 2 successful rolls
    And the raider's hp is 0
    And replaying the tick reproduces the save
