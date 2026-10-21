Feature: A played round has one commit

  Scenario: A GM commit and an idle tick write the same kind of record
    Given a docks table with a GM and "pc_rowan" seated
    When the GM queues a move for "pc_rowan" and commits
    And the operator runs tick
    Then the journal has two tick records
    And spook verify passes
    And the commit reply names the bundle path

  Scenario: A commit on a checkpoint tick closes the bundle
    Given a docks table one tick before a checkpoint
    When the GM queues a move and commits
    Then the commit reply's log path ends in ".gz"
    And spook verify passes
