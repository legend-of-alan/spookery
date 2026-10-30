Feature: The journal verifies

  Scenario: A pack that has never committed verifies
    Given a docks table
    When the operator runs verify
    Then the command exits 0
    And stdout says "verified 0 records, state_rev 0"
    And stderr is empty

  Scenario: Twelve rounds verify
    Given a docks table
    When the operator plays 12 rounds
    And the operator runs verify
    Then the command exits 0
    And stdout says the save was verified
    And stderr is empty

  Scenario: A seated session with presence changes verifies
    Given a docks table
    When two players play, a GM commits one round, and the players leave
    And the operator runs verify
    Then the command exits 0
    And stdout says the save was verified
    And stderr is empty

  Scenario: A migrate between rounds verifies
    Given a docks table
    When the operator tries a round
    And a canon file gains a byte
    And the operator runs migrate
    And the operator tries a round
    And the operator runs verify
    Then the command exits 0
    And stdout says the save was verified
    And stderr is empty

  Scenario: Startup repair verifies
    Given a docks table
    When a seated player's host is killed and restarted
    And the operator runs verify
    Then the command exits 0
    And stdout says the save was verified
    And stderr is empty

  Scenario: A changed round intent fails verify
    Given a docks table
    When the operator tries a round
    And a queued intent the round never had is written into its record
    And the operator runs verify
    Then the command exits 1
    And stderr names that record
    And stderr names the first differing save line

  Scenario: Verify leaves the pack unchanged
    Given a docks table
    When the operator tries a round
    And the pack bytes are remembered
    And the operator runs verify
    Then the pack bytes are unchanged

  Scenario: Verify from a pre-round copy
    Given a docks table
    When the pack is copied before the round
    And the operator tries a round
    And the replay base is removed
    And the operator runs verify from that copy
    Then the command exits 0
    And stdout says "verified 1 records, state_rev 1"
    And stderr is empty

  Scenario: An empty directory is not a pack
    Given an empty directory
    When the operator runs verify
    Then the command exits 1
    And stderr is NOT_IN_SPEC
