Feature: A dropped writer frees the pack

  A child spawned by another thread holds a copy of the lock handle until it
  execs. Closing the handle alone would leave the pack locked, so the writer
  unlocks it on drop.

  Scenario: A dropped writer lock frees the pack while a copy of its handle is open
    Given the writer holds a docks pack
    And a second handle to the lock file stays open, like a child that has not exec'd yet
    When the writer drops its lock
    Then a new writer can lock the pack
