Feature: Dice and arithmetic in rule numbers

  Scenario: Damage dice roll from the round seed and replay the same
    Given a docks table
    When the pack's hit adds "2d6" to grit
    And the player hits and the GM commits
    Then Rowan's grit rose by the 2d6 faces
    And the journal records the 2d6 faces
    And the GM commit reply shows the 2d6 faces
    And the player text has no dice faces
    And replaying the tick reproduces the save
    When the operator runs audit
    Then the audit exits 0
    And stdout names "2d6 →"

  Scenario: An ability modifier is computed from a meter
    Given a docks table
    When the pack's hit adds "self.grit + 2" to grit
    And the player hits and the GM commits
    Then Rowan's grit is 4

  Scenario: Advantage keeps the higher d20
    Given a docks table
    When the pack's hit adds "2d20kh1" to grit
    And the player hits and the GM commits
    Then Rowan's grit rose by the higher d20
    And the journal records both d20 faces

  Scenario: A typo fails spook check with its line
    Given a docks table
    When the pack's hit adds "2d6 +" to grit
    And the operator runs spook check
    Then the check exits 1
    And the report says "is not a number"
    And the report names the expression line
    And the report has a location for "rules/verbs.yaml"

  Scenario: An unknown meter fails spook check
    Given a docks table
    When the pack's hit adds "self.nope" to grit
    And the operator runs spook check
    Then the check exits 1
    And the report says "meter 'nope'"
    And the report has a location for "rules/verbs.yaml"

  Scenario: A world meter read fails spook check
    Given a docks table
    When the pack's hit adds "world.grit" to grit
    And the operator runs spook check
    Then the check exits 1
    And the report says "the world has no meters"
    And the report names the expression line

  Scenario: A huge dice count fails spook check
    Given a docks table
    When the pack's hit adds "100000000d6" to grit
    And the operator runs spook check
    Then the check exits 1
    And the report says "rolls more than 1000 dice"
    And the report names the expression line

  Scenario: An NPC habit rolls its effect dice
    Given a docks table
    When Mira's habit adds "1d4" to grit
    And one round plays
    Then Mira's grit rose by the 1d4 face
    And the journal records the 1d4 face

  Scenario: Dice in a condition fail spook check
    Given a docks table
    When the pack's hit requires grit of at least "1d6"
    And the operator runs spook check
    Then the check exits 1
    And the report says "cannot roll dice"
    And the report names the line holding "1d6"

  Scenario: Overflow refuses the effect and changes nothing
    Given a docks table
    When the pack's hit adds "9223372036854775807 + 1" to grit
    And the player hits and the GM commits
    Then Rowan's grit is 1
    And the journal rejects the hit as OUT_OF_RANGE
