Feature: Actions visibly matter

  Scenario: A strike shows what changed
    Given sample-d20 with player "lark" seated facing "briar"
    When lark strikes briar and the seeded roll succeeds
    Then lark sees one line naming the change to briar's hp

  Scenario: Items gained or lost show up
    Given sample-d20 with player "lark" seated facing "briar"
    When lark uses a tonic
    Then lark sees one line saying the tonic was used up

  Scenario: An unseen gain does not turn a use into a move
    Given sample-d20 with player "lark" seated facing "briar"
    When lark uses her tonic while someone out of sight gains one
    Then lark sees one line saying the tonic was used up
    And that line does not mention the unseen gain

  Scenario: Dropping an item is a move
    Given sample-d20 with player "lark" seated facing "briar"
    When lark drops the tonic
    Then lark sees one line saying she dropped the tonic

  Scenario: Taking an item is a move
    Given sample-d20 with player "lark" seated facing "briar"
    And a tonic is on the ground
    When lark takes the tonic
    Then lark sees one line saying she picked up the tonic

  Scenario: A player sees their own meters
    Given sample-d20 with player "lark" seated facing "briar"
    And lark has a hidden meter
    When lark looks at their view
    Then lark sees lark's hp and might
    And lark does not see a hidden meter

  Scenario: Changes the player may not know stay hidden
    Given sample-d20 with player "lark" seated facing "briar"
    And a round that changes a hidden fact
    Then the player's change line does not mention it
