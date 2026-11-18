Feature: The GM reads typed events

  Scenario: The GM view shows a commit's checks and dice from the typed reply
    Given a docks table with a GM and "wren" seated
    When the GM queues "exert" for "rowan" and commits
    Then the GM terminal shows one check line and one dice line for "Rowan"
    And the GM shows "Changed:"
    And the GM shows "Roll: 8 vs 0, success"
    And the GM shows "dice 2d6 → 5, 2 = 7 + 1"
    When the GM types "commit"
    Then the GM shows "Nothing changed."
