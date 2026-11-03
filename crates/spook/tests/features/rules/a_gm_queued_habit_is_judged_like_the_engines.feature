Feature: A GM-queued habit is judged like the engine's

  Scenario: A GM-queued habit is judged the same as the engine's
    Given Mira has a habit that advances the "restock" clock, which pc_rowan owns
    And pc_rowan and pc_wren are seated in the shop with Mira
    When the GM queues Mira's "mark_restock" habit and commits
    Then the round refuses the habit with "UNKNOWN_ID"
    And the "restock" clock is at 0
    And spook verify passes on this pack

  Scenario: The engine refuses the same habit with nobody watching
    Given Mira has a habit that advances the "restock" clock, which pc_rowan owns
    When an empty round passes
    Then the round refuses the habit with "UNKNOWN_ID"
    And the "restock" clock is at 0
    And spook verify passes on this pack
