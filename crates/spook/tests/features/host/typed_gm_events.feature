Feature: The GM reads typed events

  Scenario: The GM view shows a commit's checks and dice from the typed reply
    Given a docks table with a GM and "wren" seated
    When the GM queues "grit" for "rowan" and commits
    Then the GM terminal shows one check line and one dice line for "Rowan"
    And the GM shows "Changed:"
    And the GM shows "Roll: 1 + 6, +1 = 8 vs 0, success"
    When the GM types "commit"
    Then the GM shows "Nothing changed."
