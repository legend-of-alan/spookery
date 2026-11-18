Feature: Dice and arithmetic in rule numbers

  Scenario: Damage dice roll from the round seed and replay the same
    Given a docks table
    When the pack's hit adds "2d6" to vigor
    And the player hits and the GM commits
    Then Rowan's vigor rose by the 2d6 faces
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
    Then Rowan's vigor rose by the <kept> face of 2d20k

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

  Scenario Outline: An NPC habit rolls its effect dice
    Given a docks table
    When Mira's habit <adds>
    And the GM queues Aldric's wait and commits
    Then Mira's vigor rose by the 1d4 face

    Examples:
      | adds                                        |
      | adds "1d4" to vigor                         |
      | stores "lift: 1d4" and adds "lift" to vigor |

  Scenario Outline: Dice in a condition or a cost fail spook check
    Given a docks table
    And the pack has "1d6" as <field>
    When the operator runs spook check
    Then the check exits 1
    And the report says "cannot roll dice"
    And the report names the line in <file> holding "1d6"

    Examples:
      | field                     | file                       |
      | the hit's condition value | rules/verbs.yaml           |
      | the hit's cost amount     | rules/verbs.yaml           |
      | the hit's cost item count | rules/verbs.yaml           |
      | the hit's cost ticks      | rules/verbs.yaml           |
      | the street exit's ticks   | world/places/docks_shop.md |

  Scenario: Every number in a rule line can read a meter, and an exit's ticks are read for each viewer
    Given a docks table with Rowan and Wren seated in the shop
    And the rule "stock" reads Rowan's rest of 3 in every number
    And the pack has "self.assassination_debt + 3" as the street exit's ticks
    When Rowan plays "stock"
    Then Rowan's look shows "stocking until tick 1090"
    When rounds pass until tick 1090
    Then Rowan holds 4 chock
    And the fatigue has 5 ticks left
    And the street exit shows 5 ticks to Rowan and 0 to Wren, who carries no assassination_debt

  Scenario Outline: A number read out of range at play refuses the move OUT_OF_RANGE
    Given a docks table with Rowan and Wren seated in the shop
    And the pack has "<number>" as <field>
    Then Rowan's "<move>" is refused OUT_OF_RANGE

    Examples:
      | field                       | number          | move                |
      | the street exit's ticks     | self.rest - 4   | travel docks_street |
      | the strain's modifier ticks | self.rest - 3   | strain              |
      | the strain's cycle bound    | self.rest * 500 | strain              |

  Scenario: Overflow refuses the effect and changes nothing
    Given a docks table
    When the pack's hit adds "9223372036854775807 + 1" to vigor
    And the player hits and the GM commits
    Then Rowan's vigor is 1
