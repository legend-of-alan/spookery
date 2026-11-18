Feature: A rule stores a roll in let

  Scenario: A stored roll read in two places is rolled once
    Given a docks table
    When the pack's hit stores "boost: 2d6" and adds "boost" to Rowan's and Mira's vigor
    And the player hits and the GM commits
    Then Rowan's and Mira's vigor both rose by the boost total
    And the GM commit reply holds dice lines for "boost: 2d6"
    And the player text has no dice faces
    And spook verify passes

  Scenario Outline: high and low read one face of a stored pool, also from a nested check under by: target
    Given a docks table
    When the pack's hit stores "dmg: 2d8" and has "<read>" as <field>
    And the player <hits> and the GM commits
    Then Rowan's vigor rose by the <kept> face of dmg:
    And the GM commit reply holds dice lines for "dmg: 2d8"

    Examples:
      | read      | kept   | field                      | hits        |
      | high(dmg) | higher | its vigor add              | hits        |
      | low(dmg)  | lower  | its vigor add              | hits        |
      | high(dmg) | higher | a nested check's vigor add | hits Aldric |

  Scenario Outline: A rule's dice roll in the order the rule reads and an effect that does not run rolls nothing
    Given a docks table
    When the pack's hit stores "attack: 1d20", checks "attack + 1d4" against <success_at>, and adds "1d6" to vigor on a success
    And the player hits and the GM commits
    Then the GM commit reply holds dice lines for "<lines>"
    And spook verify passes

    Examples:
      | success_at | lines                  |
      | 0          | attack: 1d20, 1d4, 1d6 |
      | 100        | attack: 1d20, 1d4      |

  Scenario: A move that takes time reads its let at its finish
    Given a docks table
    When the pack's hit stores "zeal: self.rest", takes 30 ticks, and adds "zeal" to vigor
    And Mira's rouse takes 10 ticks and adds 2 to Rowan's rest
    And the player hits while the GM queues Mira's rouse and the GM commits
    Then Rowan's vigor is 6

  Scenario Outline: A second let, a let id or a read that check refuses fails spook check at the line it names
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
      | attack: 1d20 + 2      | high(attack) | its vigor add     | rule 'hit' reads high or low of 'attack', which is not one dice group with no keep          | id: hit   |
      | adv: 2d20kh1          | low(adv)     | its vigor add     | rule 'hit' reads high or low of 'adv', which is not one dice group with no keep             | id: hit   |
      | a: 1d4, a: 1d6        | a            | its vigor add     | 'a' is listed twice; keep one                                                                | a: 1d6    |
      | a: 1d4                | b: 1d6       | a second let      | 'let' is listed twice; keep one                                                              | b: 1d6    |
      | a.b: 1d4              | 1            | its vigor add     | let: id 'a.b' is not a word a number can read                                              | a.b: 1d4  |
      | self: 1d4             | 1            | its vigor add     | let: id 'self' is not a word a number can read                                             | self: 1d4 |
      | dmg: 2d8              | dmg          | a condition value | a condition or a cost cannot roll dice                                                       | op:       |
      | dmg: 2d8              | dmg          | a cost amount     | a condition or a cost cannot roll dice                                                       | meters:   |
