Feature: Each roll has its own seed

  Scenario: An unrelated intent doesn't change another intent's dice
    Given a round where Lark rolls grit
    When the same round is replayed with an extra unrelated NPC roll queued first
    Then Lark's faces and total are identical

  Scenario: Ten dice in one round
    Given ten rolling intents in one round
    Then all ten resolve and none is refused NotInSpec

  Scenario: Verify reproduces every roll
    Given a played d20 fight
    Then spook verify passes and re-derives every roll

  Scenario: Opposed strikes share one roll each
    When Lark and Briar strike each other
    Then each strike is scored on the other's own roll
