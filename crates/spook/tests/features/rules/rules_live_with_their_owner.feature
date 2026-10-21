Feature: Rules live with their owner

  Scenario: Namespaced ids
    Given a place rule "bell" in docks_shop and a story rule "bell"
    Then spook check --rules lists place:docks_shop/bell and story:smuggler/bell

  Scenario: Most specific verb rule wins
    Given verb:travel and place:ferry_dock/travel
    When a PC at ferry_dock travels
    Then place:ferry_dock/travel runs and verb:travel does not

  Scenario: Habit is a cast rule
    Given cast:briar/restock with hour 6..9 and humans == 0
    When an empty tick lands at hour 7 with briar in the shop
    Then docks_shop lamp_oil increases by 4

  Scenario: Difficulty from defense
    Given attack targets defense + 10 and briar's defense is 12
    When lark attacks with a seeded total of 21
    Then the outcome is a miss

  Scenario: Old files refused
    Given a pack with rules/moves.yaml
    Then spook check says moves.yaml was replaced by rules/verbs.yaml

  Scenario: An unknown effect key is reported on that key
    Given a verb file where the word bogus appears in earlier text
    When an effect key later in that file is bogus
    Then spook check reports that key at its own line and column

  Scenario: Two conditions under one dash are refused
    Given a verb whose if dash holds two conditions
    Then spook check reports two conditions under one dash

  Scenario: A cost the actor cannot pay changes nothing
    Given steady costs 5 grit and the sitter has 1
    When the sitter plays steady
    Then the refusal is CANT_AFFORD naming grit and world_time stays 1080

  Scenario: A full id does not select a rule
    Given a habit cast:mira/leave_door_note and a place rule place:docks_street/travel while the PC stays in the shop
    When a PC is gated on those full ids
    Then both are refused with NOT_IN_SPEC and the save is unchanged

  Scenario: An owned rule names how the next token is read
    Given use on tonic targets an actor and the shared use takes no target
    When the sitter lines up use tonic
    Then the queued target is that actor and the save is unchanged

  Scenario: A time cost is quiet at check
    Given a verb with a time cost and a resist roll
    Then spook check is quiet about the time and the resist

  Scenario: A bare resist same is reported at check
    Given a verb whose only resist is same
    Then spook check reports resist: same with no earlier roll

  Scenario: Resist same after a roll is quiet
    Given a verb whose resist same follows a roll
    Then spook check does not report a bare resist same
