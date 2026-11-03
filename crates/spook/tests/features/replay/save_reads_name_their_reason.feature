Feature: Save reads name their reason

  Scenario: A save the disk will not read is STORE_IO on migrate
    Given a docks table
    When the save cannot be read
    And the operator runs migrate
    Then the command exits 1
    And stderr is STORE_IO

  Scenario: A missing save is NOT_IN_SPEC on migrate
    Given a docks table
    When the save snapshot is removed
    And the operator runs migrate
    Then the command exits 1
    And stderr is NOT_IN_SPEC

  Scenario: A copied save the disk will not read is STORE_IO on replay
    Given a docks table
    When the pack is copied before the round
    And the operator tries a round
    And the copied save cannot be read
    And the operator runs verify from that copy
    Then the command exits 1
    And stderr is STORE_IO

  Scenario: A save the disk will not read is STORE_IO on rebuild
    Given a docks table
    And checkpoints are taken every 2 rounds
    When the operator plays 2 rounds
    And the save cannot be read
    And the operator tries a round
    Then the command exits 1
    And stderr is STORE_IO
