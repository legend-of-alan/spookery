Feature: Hash-chained journal bundles

  Scenario: A torn tail is cut off on startup
    Given a docks table
    When the operator runs 3 ticks
    And the live bundle gains a torn tail
    And the table is opened again
    Then the torn tail is gone
    And the opened state matches the last complete record

  Scenario: An edit to a closed bundle fails verification
    Given a docks table
    And checkpoints are taken every 2 ticks
    When the operator runs 4 ticks
    And one byte in a closed bundle is changed
    And the operator runs verify
    Then the command exits 1
    And stderr names that bundle

  Scenario: A missing snapshot is rebuilt from the bundles
    Given a docks table
    And checkpoints are taken every 2 ticks
    When the operator runs 4 ticks
    And the save snapshot is removed
    And the operator rebuilds the snapshot
    Then the save hash matches the latest checkpoint

  Scenario: Parallel full verification matches serial replay
    Given a docks table
    And checkpoints are taken every 2 ticks
    When the operator runs 6 ticks
    Then parallel full verification matches serial replay
