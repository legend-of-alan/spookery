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

  Scenario: An ability modifier is computed from a meter
    Given a docks table
    When the pack's hit adds "self.vigor + 2" to vigor
    And the player hits and the GM commits
    Then Rowan's vigor is 4

  Scenario: Advantage keeps the higher d20
    Given a docks table
    When the pack's hit adds "2d20kh1" to vigor
    And the player hits and the GM commits
    Then Rowan's vigor rose by the higher d20
    And the journal records both d20 faces

  Scenario Outline: A bad expression fails spook check with its line
    Given a docks table
    When the pack's hit adds "<expression>" to vigor
    And the operator runs spook check
    Then the check exits 1
    And the report says "<phrase>"
    And the report names the expression line

    Examples:
      | expression    | phrase                                           |
      | 2d6 +         | is not a number                                  |
      | self.nope     | meter 'nope'                                     |
      | world.vigor   | the world carries no meter 'vigor'               |
      | subject.vigor | names 'subject', which is not an owner; use self |
      | 100000000d6   | rolls more than 1000 dice                        |
      | 0d6           | rolls no dice                                    |
      | 10 / 0        | divides by zero                                  |

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
