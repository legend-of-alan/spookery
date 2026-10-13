Feature: Each roll has its own seed

  Scenario: An unrelated intent doesn't change another intent's dice
    Given a tick where Lark rolls grit
    When the same tick is replayed with an extra unrelated NPC roll queued first
    Then Lark's faces and total are identical

  Scenario: Ten dice in one tick
    Given ten rolling intents in one tick
    Then all ten resolve and none is refused NotInSpec

  Scenario: Verify reproduces every roll
    Given a played d20 fight
    Then spook verify passes and re-derives every roll
