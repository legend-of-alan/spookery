Feature: Hash-chained journal bundles

  Scenario: A torn tail is cut off on startup
    Given a docks table
    When the operator plays 3 rounds
    And the live bundle gains a torn tail
    And the table is opened again
    Then the torn tail is gone
    And the opened state matches the last complete record

  Scenario: Startup does not open a closed bundle
    Given a docks table
    And checkpoints are taken every 2 rounds
    When the operator plays 5 rounds
    And every closed bundle is made unreadable
    Then the table still opens

  Scenario: An edit to a closed bundle fails verification
    Given a docks table
    And checkpoints are taken every 2 rounds
    When the operator plays 4 rounds
    And one record in a closed bundle is changed
    And the operator runs verify
    Then the command exits 1
    And stderr names that bundle

  Scenario: A missing snapshot is rebuilt from the bundles
    Given a docks table
    And checkpoints are taken every 2 rounds
    When the operator plays 4 rounds
    And the save snapshot is removed
    And the operator rebuilds the snapshot
    Then the save hash matches the latest checkpoint

  Scenario: Quick verification reports the latest bundle
    Given a docks table
    And checkpoints are taken every 2 rounds
    When the operator plays 5 rounds
    And the operator runs quick verify
    Then the command exits 0
    And stdout says the save was verified

  Scenario: Quick verification fails when the live bundle is edited
    Given a docks table
    And checkpoints are taken every 2 rounds
    When the operator plays 5 rounds
    And one record in the live bundle is changed
    And the operator runs quick verify
    Then the command exits 1
    And stderr names that bundle

  Scenario: A changed checkpoint snapshot fails verification
    Given a docks table
    And checkpoints are taken every 2 rounds
    When the operator plays 6 rounds
    And the first closed checkpoint snapshot is changed
    And the operator runs verify
    Then the command exits 1
    And stderr names that bundle

  Scenario: A bundle left open after its checkpoint is closed on the next open
    Given a docks table
    And checkpoints are taken every 2 rounds
    When the operator plays 2 rounds
    And the closed bundle is expanded back to plain JSONL
    And the operator plays a round
    Then the live bundle is bundle-0001.jsonl
    And bundle-0000.jsonl is closed
    When the operator runs verify
    Then the command exits 0

  Scenario: A plain bundle beside its gzip is removed on open
    Given a docks table
    And checkpoints are taken every 2 rounds
    When the operator plays 2 rounds
    And the closed bundle is copied back beside its gzip
    And the table is opened again
    Then the plain bundle file is gone
    When the operator runs verify
    Then the command exits 0

  Scenario: A checkpoint whose save did not land is published by the next round
    Given a docks table
    And checkpoints are taken every 2 rounds
    When the operator plays a round
    And the save is kept
    And the operator plays a round
    And the checkpoint's save and snapshot are rolled back
    And the operator plays a round
    Then the save hash matches the latest checkpoint
    And the live bundle is bundle-0001.jsonl
    And bundle-0000.jsonl is closed
    When the operator runs verify
    Then the command exits 0
