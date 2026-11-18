Feature: Dice and arithmetic in rule numbers

  Scenario: Damage dice roll from the round seed and replay the same
    Given a docks table
    When the pack's hit adds "2d6" to vigor
    And the player hits and the GM commits
    Then Rowan's vigor rose by the 2d6 faces
    And the journal records the 2d6 faces
    And the GM commit reply shows the 2d6 faces
    And the player text has no dice faces
    And replaying the round reproduces the save

  Scenario Outline: A rule number reads an actor's or a place's meter, min, max and minus, and division rounds down
    Given a docks table
    When the pack's hit adds "<expression>" to vigor
    And the player hits and the GM commits
    Then Rowan's vigor is <vigor>

    Examples:
      | expression               | vigor |
      | self.vigor + 2           | 4     |
      | place:docks_shop.comfort | 3     |
      | min(self.vigor, 3)       | 2     |
      | max(self.vigor, 3)       | 4     |
      | -(self.vigor - 3)        | 3     |
      | (0 - 5) / 2 + 4          | 2     |

  Scenario Outline: Advantage keeps the higher d20 and disadvantage the lower
    Given a docks table
    When the pack's hit adds "<dice>" to vigor
    And the player hits and the GM commits
    Then Rowan's vigor rose by the <kept> d20
    And the journal records both d20 faces

    Examples:
      | dice    | kept   |
      | 2d20kh1 | higher |
      | 2d20kl1 | lower  |

  Scenario Outline: A bad expression fails spook check with its line
    Given a docks table
    When the pack's hit adds "<expression>" to vigor
    And the operator runs spook check
    Then the check exits 1
    And the report says "<phrase>"
    And the report names the expression line

    Examples:
      | expression    | phrase                                                                                                                                     |
      | 2d6 +         | expression '2d6 +' is not a number (SCHEMA)                                                                                                |
      | self.nope     | expression reads meter 'nope', but no file declares meter 'nope'; declare the meter on that entity, or read one that exists (SCHEMA)       |
      | world.vigor   | expression reads meter 'vigor', but the world carries no meter 'vigor'; declare the meter on that entity, or read one that exists (SCHEMA) |
      | subject.vigor | 'subject' is not an owner; use self, or target for the move's target (SCHEMA)                                                              |
      | exit:a.b.hp   | 'exit:a.b' is an exit, which has no meters (SCHEMA)                                                                                        |
      | 100000000d6   | expression '100000000d6' rolls more than 1000 dice (SCHEMA)                                                                                |
      | 0d6           | expression '0d6' rolls no dice (SCHEMA)                                                                                                    |
      | 10 / 0        | expression '10 / 0' divides by zero (SCHEMA)                                                                                               |

  Scenario: An owner named subject fails spook check
    Given a docks table
    When the pack's hit raises the vigor of "subject"
    And the operator runs spook check
    Then the check exits 1
    And the report says "'subject' is not an owner"
    And the report says "use self"
    And the report names the line holding "subject"

  Scenario: An NPC habit rolls its effect dice
    Given a docks table
    When Mira's habit adds "1d4" to vigor
    And one round plays
    Then Mira's vigor rose by the 1d4 face
    And the journal records the 1d4 face

  Scenario: Dice in a condition fail spook check
    Given a docks table
    When the pack's hit requires vigor of at least "1d6"
    And the operator runs spook check
    Then the check exits 1
    And the report says "cannot roll dice"
    And the report names the line holding "1d6"

  Scenario: Overflow refuses the effect and changes nothing
    Given a docks table
    When the pack's hit adds "9223372036854775807 + 1" to vigor
    And the player hits and the GM commits
    Then Rowan's vigor is 1
    And the journal rejects the hit as OUT_OF_RANGE
