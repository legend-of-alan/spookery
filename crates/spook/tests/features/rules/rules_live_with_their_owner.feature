Feature: Rules live with their owner

  Scenario: Namespaced ids
    Given a place rule "bell" in docks_shop and a story rule "bell"
    Then spook check --rules lists place:docks_shop/bell and story:smuggler/bell

  Scenario: Most specific verb rule wins
    Given verb:travel and place:ferry_dock/travel
    When a PC at ferry_dock travels
    Then place:ferry_dock/travel runs and verb:travel does not

  Scenario: Habit is a cast rule
    Given cast:briar/restock with cycle day 360..540 and players == 0
    When an empty round lands at tick 420 with briar in the shop
    Then docks_shop lamp_oil increases by 4

  Scenario: Difficulty from defense
    Given attack targets defense + 10 and briar's defense is 12
    When harbormouse attacks with a seeded total of 21
    Then the outcome is a miss

  Scenario: A pack with rules/moves.yaml fails check
    Given a pack with rules/moves.yaml
    Then spook check says moves.yaml was replaced by rules/verbs.yaml

  Scenario: An unknown effect key is reported on that key
    Given a verb file where the word bogus appears in earlier text
    When an effect key later in that file is bogus
    Then spook check reports that key at its own line and column

  Scenario Outline: Two keys in one condition or effect are refused at the second key
    Given a verb whose <field> holds two keys in one line
    Then spook check reports two keys in one condition or effect at the second key

    Examples:
      | field   |
      | if list |
      | do list |
      | if map  |

  Scenario: A cost the actor cannot pay changes nothing
    Given steady costs 5 vigor and the sitter has 1
    When the seated PC plays steady
    Then the refusal is CANT_AFFORD naming vigor and world_time stays 1080

  Scenario: A full id does not select a rule
    Given a habit cast:mira/leave_door_note and a place rule place:docks_street/travel while the PC stays in the shop
    When a PC is gated on those full ids
    Then both are refused with NOT_IN_SPEC and the save is unchanged

  Scenario: An owned rule names how the next token is read
    Given use on tonic targets an item and the shared use takes no target
    When the seated PC queues use tonic
    Then the queued target is that item and the save is unchanged

  Scenario: A time cost is quiet at check
    Given a verb with a time cost and a resist roll
    Then spook check is quiet about the time and the resist

  Scenario: A bare resist same is reported at check
    Given a verb whose only resist is same
    Then spook check reports resist: same with no earlier roll

  Scenario: Resist same after a roll is quiet
    Given a verb whose resist same follows a roll
    Then spook check does not report a bare resist same

  Scenario: A cast member's rule is at hand while they share the room
    Given briar's cast file holds a verb "ask" that targets an actor and sets the world flag passage, and Mira's door note is not due
    When harbormouse, in the yard with briar, plays ask briar
    Then cast:briar/ask runs with briar as its target and passage is set

  Scenario: A cast member's rule needs its owner named
    Given briar's cast file holds a verb "ask" that targets an actor and sets the world flag passage, and Mira's door note is not due
    When harbormouse, in the yard with briar, plays ask
    Then the move is refused NOT_IN_SPEC before the round and the save is unchanged

  Scenario: A cast member's rule refuses a target other than its owner
    Given briar's cast file holds a verb "ask" that targets an actor and sets the world flag passage, and Mira's door note is not due
    When harbormouse, in the yard with briar, plays ask harbormouse
    Then the move is refused NOT_IN_SPEC before the round and the save is unchanged

  Scenario: A cast member's rule leaves with them
    Given briar's cast file holds a verb "ask" that targets an actor, and the save puts briar in the lane
    When harbormouse, alone in the yard, plays ask briar
    Then the move is refused NOT_IN_SPEC before the round and the save is unchanged
