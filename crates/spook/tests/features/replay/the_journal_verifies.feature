Feature: The journal verifies

  Scenario: A pack that has never committed verifies
    Given a docks table
    When the operator runs verify
    Then the command exits 0
    And stdout says "verified 0 records, state_rev 0"
    And stderr is empty

  Scenario: Twelve ticks verify
    Given a docks table
    When the operator runs 12 ticks
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

  Scenario: A migrate between ticks verifies
    Given a docks table
    When the operator runs tick
    And a canon file gains a byte
    And the operator runs migrate
    And the operator runs tick
    And the operator runs verify
    Then the command exits 0
    And stdout says the save was verified
    And stderr is empty

  Scenario: Startup repair verifies
    Given a docks table
    When a seated player's server is killed and restarted
    And the operator runs verify
    Then the command exits 0
    And stdout says the save was verified
    And stderr is empty

  Scenario: A changed tick intent fails verify
    Given a docks table
    When the operator runs tick
    And one intent in a tick record is changed
    And the operator runs verify
    Then the command exits 1
    And stderr names that record
    And stderr names the first differing save line

  Scenario: A deleted presence record fails verify
    Given a docks table
    When a player sits and the server stops
    And one presence record is deleted
    And the operator runs verify
    Then the command exits 1
    And stderr names that record

  Scenario: Verify and replay leave the pack unchanged
    Given a docks table
    When the operator runs tick
    And the pack bytes are remembered
    And the operator runs verify
    And the operator runs replay
    Then the pack bytes are unchanged

  Scenario: Replay prints the final state_rev
    Given a docks table
    When the operator runs 12 ticks
    And the operator runs replay
    Then the command exits 0
    And stdout shows the final state_rev
    And stderr is empty
