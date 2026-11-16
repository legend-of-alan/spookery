Feature: Each roll has its own seed

  Scenario: An unrelated intent doesn't change another intent's dice
    Given a round where Harbormouse rolls exert
    When the same round is played again with an extra unrelated NPC roll queued first
    Then Harbormouse's faces and total are identical

  Scenario: Ten dice in one round
    Given ten rolling intents in one round
    Then all ten resolve and none is refused NotInSpec

  Scenario: Verify reproduces every roll
    Given a played d20 strike
    Then spook verify passes and re-derives every roll

  Scenario: Opposed strikes share one roll each
    When Harbormouse and Briar strike each other
    Then each strike is scored on the other's own roll

  Scenario: A fresh world picks its own seed, and spook verify replays it
    Given two fresh copies of the docks pack as shipped
    When one round is played on each
    Then the two saves hold different seeds
    And spook verify passes on each
