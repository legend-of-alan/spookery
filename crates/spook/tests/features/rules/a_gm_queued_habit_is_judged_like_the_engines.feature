Feature: A GM-queued habit is judged like the engine's

  Scenario: A GM-queued habit is judged the same as the engine's
    Given Mira has a habit that sets the "restocked" flag on her place, which its file does not list
    And rowan and wren are seated in the shop with Mira
    When the GM queues Mira's "mark_restock" habit and commits
    Then the round's only refusal is "UNKNOWN_ID"
    And no place has the "restocked" flag
    And spook verify passes on this pack

  Scenario: The engine refuses the same habit with nobody watching
    Given Mira has a habit that sets the "restocked" flag on her place, which its file does not list
    When an empty round passes
    Then the round's only refusal is "UNKNOWN_ID"
    And no place has the "restocked" flag
    And spook verify passes on this pack
