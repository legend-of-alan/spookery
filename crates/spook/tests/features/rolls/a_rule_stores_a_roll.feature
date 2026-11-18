Feature: A rule stores a roll in let

  Scenario: A stored roll read in two places is rolled once
    Given a docks table
    When the pack's hit stores "boost: 2d6" and adds "boost" to Rowan's and Mira's vigor
    And the player hits and the GM commits
    Then Rowan's and Mira's vigor both rose by the boost total
    And the journal and the GM commit reply hold dice lines for "boost: 2d6"
    And the player text has no dice faces
    And replaying the round reproduces the save

  Scenario Outline: high and low read one face of a stored pool
    Given a docks table
    When the pack's hit stores "dmg: 2d8" and has "<read>" as its vigor add
    And the player hits and the GM commits
    Then Rowan's vigor rose by the <kept> face of dmg:

    Examples:
      | read      | kept   |
      | high(dmg) | higher |
      | low(dmg)  | lower  |

  Scenario Outline: A rule's dice roll in the order the rule reads, an effect that does not run rolls nothing, and a move that takes time rolls at its finish
    Given a docks table
    When the pack's hit stores "attack: 1d20", checks "attack + 1d4" against <success_at>, takes <ticks> ticks, and adds "1d6" to vigor on a success
    And the player hits and the GM commits
    Then the journal and the GM commit reply hold dice lines for "<lines>"
    And no round before the last one rolled dice
    And spook verify passes

    Examples:
      | success_at | ticks | lines                  |
      | 0          | 0     | attack: 1d20, 1d4, 1d6 |
      | 100        | 0     | attack: 1d20, 1d4      |
      | 0          | 30    | attack: 1d20, 1d4, 1d6 |

  Scenario Outline: A let name or read that check refuses fails spook check at the line it names
    Given a docks table
    When the pack's hit stores "<let>" and has "<number>" as <field>
    And the operator runs spook check
    Then the check exits 1
    And the report says "<phrase>"
    And the report names the line holding "<line>"

    Examples:
      | let                   | number       | field             | phrase                                                                                       | line      |
      | dmg: 2d8              | dmgg         | its vigor add     | rule 'hit' reads 'dmgg', which its let: does not hold                                        | id: hit   |
      | soon: late, late: 2d6 | soon         | its vigor add     | rule 'hit' reads 'late' in let: at or above the line that holds it                           | id: hit   |
      | attack: 1d20 + 2      | high(attack) | its vigor add     | rule 'hit' reads high or low of 'attack', which is not one dice group that keeps every face | id: hit   |
      | adv: 2d20kh1          | low(adv)     | its vigor add     | rule 'hit' reads high or low of 'adv', which is not one dice group that keeps every face    | id: hit   |
      | a: 1d4, a: 1d6        | a            | its vigor add     | SCHEMA                                                                                       | a: 1d6    |
      | a.b: 1d4              | 1            | its vigor add     | let: name 'a.b' is not a word a number can read                                              | a.b: 1d4  |
      | self: 1d4             | 1            | its vigor add     | let: name 'self' is not a word a number can read                                             | self: 1d4 |
      | dmg: 2d8              | dmg          | a condition value | a condition or a cost cannot roll dice                                                       | op:       |
      | dmg: 2d8              | dmg          | a cost amount     | a condition or a cost cannot roll dice                                                       | meters:   |
