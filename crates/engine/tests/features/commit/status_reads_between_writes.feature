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

  Scenario: Status reads a pack the writer holds between rounds
    Given the writer holds a docks pack
    And the writer has committed a round
    When status peeks the pack
    Then status prints the round the writer committed

  Scenario: A second writer is still refused
    Given the writer holds a docks pack
    When a second writer locks the pack
    Then the second writer is refused with COMMIT_CONFLICT
