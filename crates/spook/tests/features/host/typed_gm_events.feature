Feature: The GM reads typed events

  Scenario: The GM view shows a commit's checks and dice from the typed reply
    Given a docks table with a GM and "pc_rowan" seated
    When the GM queues "grit" for "pc_rowan" and commits
    Then the GM terminal shows one check line and one dice line for "pc_rowan"

  Scenario: A body that matches no known type is reported, not dropped
    Given an event reader
    When the reader is given an event body that matches no known type
    Then the reader reports a bad event
