Feature: The GM reads typed events

  Scenario: The GM view shows a commit's checks and dice from the typed reply
    Given a docks table with a GM and "pc_rowan" seated
    When the GM queues "grit" for "pc_rowan" and commits
    Then the GM terminal shows one check line and one dice line for "pc_rowan"
