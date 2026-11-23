Feature: A GM-queued habit is judged like the engine's

  Scenario: A GM-queued habit is judged the same as the engine's
    Given Mira has a habit that adds 1 to her "shop_day" meter on her place, which only she carries
    And rowan and wren are seated in the shop with Mira
    When the GM queues Mira's "mark_restock" habit and commits
    Then no place has a "shop_day" meter
    And spook verify passes on this pack

  Scenario: The engine refuses the same habit with nobody watching
    Given Mira has a habit that adds 1 to her "shop_day" meter on her place, which only she carries
    When an empty round passes
    Then no place has a "shop_day" meter
    And spook verify passes on this pack
