Feature: Status reads the pack between writes

  The writer holds state/.pack.lock while it has the pack open, and
  state/.pack.rw.lock only while a write publishes. Status holds a shared lock
  on state/.pack.rw.lock while it reads, so it waits out a write in progress.
  The steps redo the write by hand: holding the write guard, they put the files
  back as they stood halfway through, start status, then finish the write.

  Scenario: Status during a commit sees a whole round
    Given the writer holds a docks pack
    When status peeks while the writer is halfway through appending a round
    Then status waited for the write to finish
    And status prints the round the writer committed

  Scenario: Status during a checkpoint sees a matching set
    Given the writer holds a docks pack that checkpoints every round
    When status peeks while the writer is halfway through publishing a checkpoint
    Then status waited for the write to finish
    And status prints the round the writer committed

